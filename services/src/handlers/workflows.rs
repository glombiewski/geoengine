use std::collections::HashSet;

use crate::datasets::provenance::ProvenanceProvider;
use crate::datasets::storage::{AddDataset, DatasetDefinition, DatasetStore, MetaDataDefinition};
use crate::datasets::upload::{UploadId, UploadRootPath};
use crate::error;
use crate::error::Result;
use crate::handlers::{authenticate, Context};
use crate::util::user_input::UserInput;
use crate::util::IdResponse;
use crate::workflows::registry::WorkflowRegistry;
use crate::workflows::workflow::{Workflow, WorkflowId};
use futures::future::join_all;
use geoengine_datatypes::dataset::{DatasetId, InternalDatasetId};
use geoengine_datatypes::primitives::AxisAlignedRectangle;
use geoengine_datatypes::raster::GeoTransform;
use geoengine_datatypes::spatial_reference::SpatialReference;
use geoengine_datatypes::util::Identifier;
use geoengine_operators::engine::{OperatorDatasets, RasterQueryRectangle, TypedResultDescriptor};
use geoengine_operators::source::{
    FileNotFoundHandling, GdalDatasetParameters, GdalMetaDataStatic,
};
use geoengine_operators::util::raster_stream_to_geotiff::raster_stream_to_geotiff;
use geoengine_operators::{call_on_generic_raster_processor_gdal_types, call_on_typed_operator};
use serde::{Deserialize, Serialize};
use snafu::ResultExt;
use tokio::fs;
use uuid::Uuid;
use warp::reply::Reply;
use warp::Filter;

/// Registers a new [Workflow].
///
/// # Example
///
/// ```text
/// POST /workflow
/// Authorization: Bearer e9da345c-b1df-464b-901c-0335a0419227
///
/// {
///   "type": "Vector",
///   "operator": {
///     "type": "MockPointSource",
///     "params": {
///       "points": [
///         { "x": 0.0, "y": 0.1 },
///         { "x": 1.0, "y": 1.1 }
///       ]
///     }
///   }
/// }
/// ```
/// Response:
/// ```text
/// {
///   "id": "cee25e8c-18a0-5f1b-a504-0bc30de21e06"
/// }
/// ```
pub(crate) fn register_workflow_handler<C: Context>(
    ctx: C,
) -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    warp::path!("workflow")
        .and(warp::post())
        .and(authenticate(ctx.clone()))
        .and(warp::any().map(move || ctx.clone()))
        .and(warp::body::json())
        .and_then(register_workflow)
}

// TODO: move into handler once async closures are available?
async fn register_workflow<C: Context>(
    _session: C::Session,
    ctx: C,
    workflow: Workflow,
) -> Result<impl warp::Reply, warp::Rejection> {
    let id = ctx
        .workflow_registry_ref_mut()
        .await
        .register(workflow)
        .await?;
    Ok(warp::reply::json(&IdResponse::from(id)))
}

/// Retrieves an existing [Workflow] using its id.
///
/// # Example
///
/// ```text
/// GET /workflow/cee25e8c-18a0-5f1b-a504-0bc30de21e06
/// Authorization: Bearer e9da345c-b1df-464b-901c-0335a0419227
/// ```
/// Response:
/// ```text
/// {
///   "type": "Vector",
///   "operator": {
///     "type": "MockPointSource",
///     "params": {
///       "points": [
///         {
///           "x": 0.0,
///           "y": 0.1
///         },
///         {
///           "x": 1.0,
///           "y": 1.1
///         }
///       ]
///     }
///   }
/// }
/// ```
pub(crate) fn load_workflow_handler<C: Context>(
    ctx: C,
) -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    warp::path!("workflow" / Uuid)
        .and(warp::get())
        .and(authenticate(ctx.clone()))
        .and(warp::any().map(move || ctx.clone()))
        .and_then(load_workflow)
}

// TODO: move into handler once async closures are available?
async fn load_workflow<C: Context>(
    id: Uuid,
    _session: C::Session,
    ctx: C,
) -> Result<impl warp::Reply, warp::Rejection> {
    let wf = ctx
        .workflow_registry_ref()
        .await
        .load(&WorkflowId(id))
        .await?;
    Ok(warp::reply::json(&wf).into_response())
}

/// Gets the metadata of a workflow.
///
/// # Example
///
/// ```text
/// GET /workflow/cee25e8c-18a0-5f1b-a504-0bc30de21e06/metadata
/// Authorization: Bearer e9da345c-b1df-464b-901c-0335a0419227
/// ```
/// Response:
/// ```text
/// {
///   "dataType": "MultiPoint",
///   "spatialReference": "EPSG:4326",
///   "columns": {}
/// }
/// ```
pub(crate) fn get_workflow_metadata_handler<C: Context>(
    ctx: C,
) -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    warp::get()
        .and(warp::path!("workflow" / Uuid / "metadata"))
        .and(authenticate(ctx.clone()))
        .and(warp::any().map(move || ctx.clone()))
        .and_then(get_workflow_metadata)
}

// TODO: move into handler once async closures are available?
async fn get_workflow_metadata<C: Context>(
    id: Uuid,
    session: C::Session,
    ctx: C,
) -> Result<impl warp::Reply, warp::Rejection> {
    let workflow = ctx
        .workflow_registry_ref()
        .await
        .load(&WorkflowId(id))
        .await?;

    let execution_context = ctx.execution_context(session)?;

    // TODO: use cache here
    let result_descriptor: TypedResultDescriptor = call_on_typed_operator!(
        workflow.operator,
        operator => {
            let operator = operator
                .initialize(&execution_context).await
                .context(error::Operator)?;

            #[allow(clippy::clone_on_copy)]
            operator.result_descriptor().clone().into()
        }
    );

    Ok(warp::reply::json(&result_descriptor))
}

/// Gets the provenance of all datasets used in a workflow.
///
/// # Example
///
/// ```text
/// GET /workflow/cee25e8c-18a0-5f1b-a504-0bc30de21e06/provenance
/// Authorization: Bearer e9da345c-b1df-464b-901c-0335a0419227
/// ```
/// Response:
/// ```text
/// [{
///   "id": {
///     "type": "internal",
///     "datasetId": "846a823a-6859-4b94-ab0a-c1de80f593d8"
///   },
///   "citation": "Author, Dataset Tile",
///   "license": "Some license",
///   "uri": "http://example.org/"
/// }, {
///   "id": {
///     "type": "internal",
///     "datasetId": "453cd398-f271-437b-9c3d-7f42213ea30a"
///   },
///   "citation": "Another Author, Another Dataset Tile",
///   "license": "Some other license",
///   "uri": "http://example.org/"
/// }]
/// ```
pub(crate) fn get_workflow_provenance_handler<C: Context>(
    ctx: C,
) -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    warp::get()
        .and(warp::path!("workflow" / Uuid / "provenance"))
        .and(authenticate(ctx.clone()))
        .and(warp::any().map(move || ctx.clone()))
        .and_then(get_workflow_provenance)
}

// TODO: move into handler once async closures are available?
async fn get_workflow_provenance<C: Context>(
    id: Uuid,
    _session: C::Session,
    ctx: C,
) -> Result<impl warp::Reply, warp::Rejection> {
    let workflow = ctx
        .workflow_registry_ref()
        .await
        .load(&WorkflowId(id))
        .await?;

    let datasets = workflow.operator.datasets();

    let db = ctx.dataset_db_ref().await;

    let provenance: Vec<_> = datasets.iter().map(|id| db.provenance(id)).collect();
    let provenance: Result<Vec<_>> = join_all(provenance).await.into_iter().collect();

    // filter duplicates
    let provenance: HashSet<_> = provenance?.into_iter().collect();
    let provenance: Vec<_> = provenance.into_iter().collect();

    Ok(warp::reply::json(&provenance))
}

/// parameter for the dataset from workflow handler (body)
#[derive(Clone, Debug, Deserialize, Serialize)]
struct RasterDatasetFromWorkflow {
    name: String,
    description: Option<String>,
    query: RasterQueryRectangle,
}

/// response of the dataset from workflow handler
#[derive(Clone, Debug, Deserialize, Serialize)]
struct RasterDatasetFromWorkflowResult {
    dataset: DatasetId,
    upload: UploadId,
}

/// Create a new dataset from the result of the given workflow and query
/// Returns the id of the created dataset and upload
///
/// # Example
///
/// ```text
/// POST /datasetFromWorkflow/{workflow_id}
/// Authorization: Bearer fc9b5dc2-a1eb-400f-aeed-a7845d9935c9
/// Content-Type: application/json
///
/// {
///     "name": "foo",
///     "description": null,
///     "query": {
///         "spatial_bounds": {
///             "upperLeftCoordinate": {
///                 "x": -10.0,
///                 "y": 80.0
///             },
///             "lowerRightCoordinate": {
///                 "x": 50.0,
///                 "y": 20.0
///             }
///         },
///         "time_interval": {
///             "start": 1388534400000,
///             "end": 1388534401000
///         },
///         "spatial_resolution": {
///             "x": 0.1,
///             "y": 0.1
///         }
///     }
/// }
///
/// ```text
/// {
///   "upload": "3086f494-d5a4-4b51-a14b-3b29f8bf7bb0",
///   "dataset": {
///     "type": "internal",
///     "datasetId": "94230f0b-4e8a-4cba-9adc-3ace837fe5d4"
///   }
/// }
/// ```
pub(crate) fn dataset_from_workflow_handler<C: Context>(
    ctx: C,
) -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    // TODO: support vector datasets

    warp::path!("datasetFromWorkflow" / Uuid)
        .map(WorkflowId)
        .and(warp::post())
        .and(authenticate(ctx.clone()))
        .and(warp::any().map(move || ctx.clone()))
        .and(warp::body::json())
        .and_then(dataset_from_workflow)
}

// TODO: move into handler once async closures are available?
async fn dataset_from_workflow<C: Context>(
    workflow_id: WorkflowId,
    session: C::Session,
    ctx: C,
    info: RasterDatasetFromWorkflow,
) -> Result<impl warp::Reply, warp::Rejection> {
    // TODO: support datasets with multiple time steps

    let workflow = ctx.workflow_registry_ref().await.load(&workflow_id).await?;

    let operator = workflow.operator.get_raster().context(error::Operator)?;

    let execution_context = ctx.execution_context(session.clone())?;
    let initialized = operator
        .clone()
        .initialize(&execution_context)
        .await
        .context(error::Operator)?;

    let result_descriptor = initialized.result_descriptor();

    let processor = initialized.query_processor().context(error::Operator)?;

    // put the created data into a new upload
    let upload = UploadId::new();
    let upload_path = upload.root_path()?;
    fs::create_dir_all(&upload_path).await.context(error::Io)?;
    let file_path = upload_path.join("raster.tiff");

    let query_rect = info.query;
    let query_ctx = ctx.query_context()?;
    let no_data_value = result_descriptor.no_data_value;
    let request_spatial_ref = Option::<SpatialReference>::from(result_descriptor.spatial_reference)
        .ok_or(error::Error::MissingSpatialReference)?;
    let tile_limit = None; // TODO: set a reasonable limit or make configurable?

    // build the geotiff
    call_on_generic_raster_processor_gdal_types!(processor, p =>  raster_stream_to_geotiff(
            &file_path,
            p,
            query_rect,
            query_ctx,
            no_data_value,
            request_spatial_ref,
            tile_limit,    
        ).await)?
    .map_err(error::Error::from)?;

    // create the dataset
    let dataset = create_dataset(info, file_path, result_descriptor, ctx, session).await?;

    Ok(warp::reply::json(&RasterDatasetFromWorkflowResult {
        dataset,
        upload,
    }))
}

async fn create_dataset<C: Context>(
    info: RasterDatasetFromWorkflow,
    file_path: std::path::PathBuf,
    result_descriptor: &geoengine_operators::engine::RasterResultDescriptor,
    ctx: C,
    session: <C as Context>::Session,
) -> Result<geoengine_datatypes::dataset::DatasetId> {
    let dataset_id = InternalDatasetId::new().into();
    let dataset_definition = DatasetDefinition {
        properties: AddDataset {
            id: Some(dataset_id),
            name: info.name,
            description: info.description.unwrap_or_default(),
            source_operator: "GdalSource".to_owned(),
            symbology: None,  // TODO add symbology?
            provenance: None, // TODO add provenance that references the workflow
        },
        meta_data: MetaDataDefinition::GdalStatic(GdalMetaDataStatic {
            time: Some(info.query.time_interval),
            params: GdalDatasetParameters {
                file_path,
                rasterband_channel: 1,
                geo_transform: GeoTransform::new(
                    info.query.spatial_bounds.upper_left(),
                    info.query.spatial_resolution.x,
                    -info.query.spatial_resolution.y,
                ),
                width: (info.query.spatial_bounds.size_x() / info.query.spatial_resolution.x).ceil()
                    as usize,
                height: (info.query.spatial_bounds.size_y() / info.query.spatial_resolution.y)
                    .ceil() as usize,
                file_not_found_handling: FileNotFoundHandling::Error,
                no_data_value: result_descriptor.no_data_value,
                properties_mapping: None, // TODO: add properties
                gdal_open_options: None,
                gdal_config_options: None,
            },
            result_descriptor: result_descriptor.clone(),
        }),
    };

    // TODO: build pyramides, prefereably in the background

    let mut db = ctx.dataset_db_ref_mut().await;
    let meta = db.wrap_meta_data(dataset_definition.meta_data);
    let dataset = db
        .add_dataset(&session, dataset_definition.properties.validated()?, meta)
        .await?;
    Ok(dataset)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contexts::{InMemoryContext, Session, SimpleContext};
    use crate::handlers::{handle_rejection, ErrorResponse};
    use crate::util::tests::{
        add_ndvi_to_datasets, check_allowed_http_methods, check_allowed_http_methods2,
        register_ndvi_workflow_helper, TestDataUploads,
    };
    use crate::util::IdResponse;
    use crate::workflows::registry::WorkflowRegistry;
    use geoengine_datatypes::collections::MultiPointCollection;
    use geoengine_datatypes::primitives::{
        FeatureData, Measurement, MultiPoint, SpatialPartition2D, SpatialResolution, TimeInterval,
    };
    use geoengine_datatypes::raster::RasterDataType;
    use geoengine_datatypes::spatial_reference::SpatialReference;
    use geoengine_operators::engine::{MultipleRasterSources, PlotOperator, TypedOperator};
    use geoengine_operators::engine::{RasterOperator, RasterResultDescriptor, VectorOperator};
    use geoengine_operators::mock::{
        MockFeatureCollectionSource, MockPointSource, MockPointSourceParams, MockRasterSource,
        MockRasterSourceParams,
    };
    use geoengine_operators::plot::{Statistics, StatisticsParams};
    use geoengine_operators::source::{GdalSource, GdalSourceParameters};
    use geoengine_operators::util::raster_stream_to_geotiff::raster_stream_to_geotiff_bytes;
    use serde_json::json;
    use warp::http::Response;
    use warp::hyper::body::Bytes;

    async fn register_test_helper(method: &str) -> Response<Bytes> {
        let ctx = InMemoryContext::default();

        let session = ctx.default_session_ref().await;

        let workflow = Workflow {
            operator: MockPointSource {
                params: MockPointSourceParams {
                    points: vec![(0.0, 0.1).into(), (1.0, 1.1).into()],
                },
            }
            .boxed()
            .into(),
        };

        // insert workflow
        warp::test::request()
            .method(method)
            .path("/workflow")
            .header("Content-Length", "0")
            .header(
                "Authorization",
                format!("Bearer {}", session.id().to_string()),
            )
            .json(&workflow)
            .reply(&register_workflow_handler(ctx.clone()).recover(handle_rejection))
            .await
    }

    #[tokio::test]
    async fn register() {
        let res = register_test_helper("POST").await;

        assert_eq!(res.status(), 200);

        let body: String = String::from_utf8(res.body().to_vec()).unwrap();
        let _id: IdResponse<WorkflowId> = serde_json::from_str(&body).unwrap();
    }

    #[tokio::test]
    async fn register_invalid_method() {
        check_allowed_http_methods(register_test_helper, &["POST"]).await;
    }

    #[tokio::test]
    async fn register_missing_header() {
        let ctx = InMemoryContext::default();

        let workflow = Workflow {
            operator: MockPointSource {
                params: MockPointSourceParams {
                    points: vec![(0.0, 0.1).into(), (1.0, 1.1).into()],
                },
            }
            .boxed()
            .into(),
        };

        // insert workflow
        let res = warp::test::request()
            .method("POST")
            .path("/workflow")
            .header("Content-Length", "0")
            .json(&workflow)
            .reply(&register_workflow_handler(ctx).recover(handle_rejection))
            .await;

        ErrorResponse::assert(
            &res,
            401,
            "MissingAuthorizationHeader",
            "Header with authorization token not provided.",
        );
    }

    #[tokio::test]
    async fn register_invalid_body() {
        let ctx = InMemoryContext::default();

        let session_id = ctx.default_session_ref().await.id();

        // insert workflow
        let res = warp::test::request()
            .method("POST")
            .path("/workflow")
            .header("Content-Length", "0")
            .header(
                "Authorization",
                format!("Bearer {}", session_id.to_string()),
            )
            .body("no json")
            .reply(&register_workflow_handler(ctx).recover(handle_rejection))
            .await;

        ErrorResponse::assert(
            &res,
            400,
            "BodyDeserializeError",
            "expected ident at line 1 column 2",
        );
    }

    #[tokio::test]
    async fn register_missing_fields() {
        let ctx = InMemoryContext::default();

        let session_id = ctx.default_session_ref().await.id();

        let workflow = json!({});

        // insert workflow
        let res = warp::test::request()
            .method("POST")
            .path("/workflow")
            .header("Content-Length", "0")
            .header(
                "Authorization",
                format!("Bearer {}", session_id.to_string()),
            )
            .json(&workflow)
            .reply(&register_workflow_handler(ctx).recover(handle_rejection))
            .await;

        ErrorResponse::assert(
            &res,
            400,
            "BodyDeserializeError",
            "missing field `type` at line 1 column 2",
        );
    }

    async fn load_test_helper(method: &str) -> (Workflow, Response<Bytes>) {
        let ctx = InMemoryContext::default();

        let session_id = ctx.default_session_ref().await.id();

        let (workflow, id) = register_ndvi_workflow_helper(&ctx).await;

        let res = warp::test::request()
            .method(method)
            .path(&format!("/workflow/{}", id.to_string()))
            .header(
                "Authorization",
                format!("Bearer {}", session_id.to_string()),
            )
            .reply(&load_workflow_handler(ctx.clone()).recover(handle_rejection))
            .await;

        (workflow, res)
    }

    #[tokio::test]
    async fn load() {
        let (workflow, res) = load_test_helper("GET").await;

        assert_eq!(res.status(), 200);
        assert_eq!(res.body(), &serde_json::to_string(&workflow).unwrap());
    }

    #[tokio::test]
    async fn load_invalid_method() {
        check_allowed_http_methods2(load_test_helper, &["GET"], |(_, res)| res).await;
    }

    #[tokio::test]
    async fn load_missing_header() {
        let ctx = InMemoryContext::default();

        let (_, id) = register_ndvi_workflow_helper(&ctx).await;

        let res = warp::test::request()
            .method("GET")
            .path(&format!("/workflow/{}", id.to_string()))
            .reply(&load_workflow_handler(ctx).recover(handle_rejection))
            .await;

        ErrorResponse::assert(
            &res,
            401,
            "MissingAuthorizationHeader",
            "Header with authorization token not provided.",
        );
    }

    #[tokio::test]
    async fn load_not_exist() {
        let ctx = InMemoryContext::default();

        let res = warp::test::request()
            .method("GET")
            .path("/workflow/1")
            .reply(&load_workflow_handler(ctx).recover(handle_rejection))
            .await;

        ErrorResponse::assert(&res, 404, "NotFound", "Not Found");
    }

    async fn vector_metadata_test_helper(method: &str) -> Response<Bytes> {
        let ctx = InMemoryContext::default();

        let session = ctx.default_session_ref().await;

        let workflow = Workflow {
            operator: MockFeatureCollectionSource::single(
                MultiPointCollection::from_data(
                    MultiPoint::many(vec![(0.0, 0.1)]).unwrap(),
                    vec![TimeInterval::default()],
                    [
                        ("foo".to_string(), FeatureData::Float(vec![42.0])),
                        ("bar".to_string(), FeatureData::Int(vec![23])),
                    ]
                    .iter()
                    .cloned()
                    .collect(),
                )
                .unwrap(),
            )
            .boxed()
            .into(),
        };

        let id = ctx
            .workflow_registry()
            .write()
            .await
            .register(workflow.clone())
            .await
            .unwrap();

        warp::test::request()
            .method(method)
            .path(&format!("/workflow/{}/metadata", id.to_string()))
            .header(
                "Authorization",
                format!("Bearer {}", session.id().to_string()),
            )
            .reply(&get_workflow_metadata_handler(ctx.clone()).recover(handle_rejection))
            .await
    }

    #[tokio::test]
    async fn vector_metadata() {
        let res = vector_metadata_test_helper("GET").await;

        assert_eq!(res.status(), 200, "{:?}", res.body());

        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(res.body()).unwrap(),
            json!({
                "type": "vector",
                "dataType": "MultiPoint",
                "spatialReference": "EPSG:4326",
                "columns": {
                    "bar": "int",
                    "foo": "float"
                }
            })
        );
    }

    #[tokio::test]
    async fn raster_metadata() {
        let ctx = InMemoryContext::default();

        let session_id = ctx.default_session_ref().await.id();

        let workflow = Workflow {
            operator: MockRasterSource {
                params: MockRasterSourceParams {
                    data: vec![],
                    result_descriptor: RasterResultDescriptor {
                        data_type: RasterDataType::U8,
                        spatial_reference: SpatialReference::epsg_4326().into(),
                        measurement: Measurement::Continuous {
                            measurement: "radiation".to_string(),
                            unit: None,
                        },
                        no_data_value: None,
                    },
                },
            }
            .boxed()
            .into(),
        };

        let id = ctx
            .workflow_registry()
            .write()
            .await
            .register(workflow.clone())
            .await
            .unwrap();

        let res = warp::test::request()
            .method("GET")
            .path(&format!("/workflow/{}/metadata", id.to_string()))
            .header(
                "Authorization",
                format!("Bearer {}", session_id.to_string()),
            )
            .reply(&get_workflow_metadata_handler(ctx))
            .await;

        assert_eq!(res.status(), 200, "{:?}", res.body());

        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(res.body()).unwrap(),
            serde_json::json!({
                "type": "raster",
                "dataType": "U8",
                "spatialReference": "EPSG:4326",
                "measurement": {
                    "type": "continuous",
                    "measurement": "radiation",
                    "unit": null
                },
                "noDataValue": null
            })
        );
    }

    #[tokio::test]
    async fn metadata_invalid_method() {
        check_allowed_http_methods(vector_metadata_test_helper, &["GET"]).await;
    }

    #[tokio::test]
    async fn metadata_missing_header() {
        let ctx = InMemoryContext::default();

        let workflow = Workflow {
            operator: MockFeatureCollectionSource::single(
                MultiPointCollection::from_data(
                    MultiPoint::many(vec![(0.0, 0.1)]).unwrap(),
                    vec![TimeInterval::default()],
                    [
                        ("foo".to_string(), FeatureData::Float(vec![42.0])),
                        ("bar".to_string(), FeatureData::Int(vec![23])),
                    ]
                    .iter()
                    .cloned()
                    .collect(),
                )
                .unwrap(),
            )
            .boxed()
            .into(),
        };

        let id = ctx
            .workflow_registry()
            .write()
            .await
            .register(workflow.clone())
            .await
            .unwrap();

        let res = warp::test::request()
            .method("GET")
            .path(&format!("/workflow/{}/metadata", id.to_string()))
            .reply(&get_workflow_metadata_handler(ctx).recover(handle_rejection))
            .await;

        ErrorResponse::assert(
            &res,
            401,
            "MissingAuthorizationHeader",
            "Header with authorization token not provided.",
        );
    }

    #[tokio::test]
    async fn plot_metadata() {
        let ctx = InMemoryContext::default();

        let session_id = ctx.default_session_ref().await.id();

        let workflow = Workflow {
            operator: Statistics {
                params: StatisticsParams {},
                sources: MultipleRasterSources { rasters: vec![] },
            }
            .boxed()
            .into(),
        };

        let id = ctx
            .workflow_registry()
            .write()
            .await
            .register(workflow.clone())
            .await
            .unwrap();

        let res = warp::test::request()
            .method("GET")
            .path(&format!("/workflow/{}/metadata", id.to_string()))
            .header(
                "Authorization",
                format!("Bearer {}", session_id.to_string()),
            )
            .reply(&get_workflow_metadata_handler(ctx))
            .await;

        assert_eq!(res.status(), 200, "{:?}", res.body());

        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(res.body()).unwrap(),
            serde_json::json!({
                "type": "plot",
            })
        );
    }

    #[tokio::test]
    async fn provenance() {
        let ctx = InMemoryContext::default();

        let session_id = ctx.default_session_ref().await.id();

        let dataset = add_ndvi_to_datasets(&ctx).await;

        let workflow = Workflow {
            operator: TypedOperator::Raster(
                GdalSource {
                    params: GdalSourceParameters {
                        dataset: dataset.clone(),
                    },
                }
                .boxed(),
            ),
        };

        let id = ctx
            .workflow_registry()
            .write()
            .await
            .register(workflow.clone())
            .await
            .unwrap();

        let res = warp::test::request()
            .method("GET")
            .path(&format!("/workflow/{}/provenance", id.to_string()))
            .header(
                "Authorization",
                format!("Bearer {}", session_id.to_string()),
            )
            .reply(&get_workflow_provenance_handler(ctx))
            .await;

        assert_eq!(res.status(), 200, "{:?}", res.body());

        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(res.body()).unwrap(),
            serde_json::json!([{
                "dataset": {
                    "type": "internal",
                    "datasetId": dataset.internal().unwrap().to_string()
                },
                "provenance": {
                    "citation": "Sample Citation",
                    "license": "Sample License",
                    "uri": "http://example.org/"
                }
            }])
        );
    }

    #[tokio::test]
    async fn dataset_from_workflow() {
        let ctx = InMemoryContext::default();

        let session_id = ctx.default_session_ref().await.id();

        let dataset = add_ndvi_to_datasets(&ctx).await;

        let workflow = Workflow {
            operator: TypedOperator::Raster(
                GdalSource {
                    params: GdalSourceParameters {
                        dataset: dataset.clone(),
                    },
                }
                .boxed(),
            ),
        };

        let workflow_id = ctx
            .workflow_registry_ref_mut()
            .await
            .register(workflow)
            .await
            .unwrap();

        // create dataset from workflow
        let res = warp::test::request()
            .method("POST")
            .path(&format!("/datasetFromWorkflow/{}", workflow_id.to_string(),))
            .header(
                "Authorization",
                format!("Bearer {}", session_id.to_string()),
            )
            .body(
                r#"{
                "name": "foo",
                "description": null,
                "query": {
                    "spatial_bounds": {
                        "upperLeftCoordinate": {
                            "x": -10.0,
                            "y": 80.0
                        },
                        "lowerRightCoordinate": {
                            "x": 50.0,
                            "y": 20.0
                        }
                    },
                    "time_interval": {
                        "start": 1388534400000,
                        "end": 1388534401000
                    },
                    "spatial_resolution": {
                        "x": 0.1,
                        "y": 0.1
                    }
                }
            }"#,
            )
            .reply(&dataset_from_workflow_handler(ctx.clone()))
            .await;

        assert_eq!(res.status(), 200);

        let body = std::str::from_utf8(res.body()).unwrap();
        let response = serde_json::from_str::<RasterDatasetFromWorkflowResult>(body).unwrap();
        // automatically deletes uploads on drop
        let _test_uploads = TestDataUploads {
            uploads: vec![response.upload],
        };

        // query the newly created dataset
        let op = GdalSource {
            params: GdalSourceParameters {
                dataset: response.dataset.clone(),
            },
        }
        .boxed();

        let session = ctx.default_session_ref().await.clone();
        let exe_ctx = ctx.execution_context(session).unwrap();

        let o = op.initialize(&exe_ctx).await.unwrap();

        let query_ctx = ctx.query_context().unwrap();
        let query_rect = RasterQueryRectangle {
            spatial_bounds: SpatialPartition2D::new((-10., 80.).into(), (50., 20.).into()).unwrap(),
            time_interval: TimeInterval::new_unchecked(1_388_534_400_000, 1_388_534_400_000 + 1000),
            spatial_resolution: SpatialResolution::zero_point_one(),
        };

        let processor = o.query_processor().unwrap().get_u8().unwrap();

        let result = raster_stream_to_geotiff_bytes(
            processor,
            query_rect,
            query_ctx,
            Some(0.),
            SpatialReference::epsg_4326(),
            None,
        )
        .await
        .unwrap();

        assert_eq!(
            include_bytes!("../../../test_data/raster/geotiff_from_stream_compressed.tiff")
                as &[u8],
            result
        );
    }
}
