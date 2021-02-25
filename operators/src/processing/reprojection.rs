use std::marker::PhantomData;

use futures::TryStreamExt;
use geoengine_datatypes::{
    operations::reproject::{CoordinateProjection, CoordinateProjector, Reproject},
    spatial_reference::SpatialReference,
};
use serde::{Deserialize, Serialize};
use snafu::ensure;

use crate::{
    engine::{
        ExecutionContext, InitializedOperator, InitializedOperatorImpl, InitializedVectorOperator,
        Operator, QueryContext, QueryRectangle, TypedVectorQueryProcessor, VectorOperator,
        VectorQueryProcessor, VectorResultDescriptor,
    },
    error,
    util::Result,
};

use super::map_query::MapQueryProcessor;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ReprojectionParams {
    pub target_spatial_reference: SpatialReference,
}

pub type Reprojection = Operator<ReprojectionParams>;
pub type InitializedReprojection =
    InitializedOperatorImpl<(), VectorResultDescriptor, (SpatialReference, SpatialReference)>;

#[typetag::serde]
impl VectorOperator for Reprojection {
    fn initialize(
        self: Box<Self>,
        context: &dyn ExecutionContext,
    ) -> Result<Box<InitializedVectorOperator>> {
        ensure!(
            self.vector_sources.len() == 1,
            error::InvalidNumberOfVectorInputs {
                expected: 1..2,
                found: self.vector_sources.len()
            }
        );
        ensure!(
            self.raster_sources.is_empty(),
            error::InvalidNumberOfRasterInputs {
                expected: 0..1,
                found: self.raster_sources.len()
            }
        );

        let initialized_vector_sources = self
            .vector_sources
            .into_iter()
            .map(|o| o.initialize(context))
            .collect::<Result<Vec<Box<InitializedVectorOperator>>>>()?;

        let in_desc: &VectorResultDescriptor = initialized_vector_sources[0].result_descriptor();
        let out_desc = VectorResultDescriptor {
            spatial_reference: self.params.target_spatial_reference.into(),
            data_type: in_desc.data_type,
            columns: in_desc.columns.clone(),
        };

        let state = (
            Option::from(in_desc.spatial_reference).unwrap(),
            self.params.target_spatial_reference,
        );

        Ok(
            InitializedReprojection::new((), out_desc, vec![], initialized_vector_sources, state)
                .boxed(),
        )
    }
}

impl InitializedOperator<VectorResultDescriptor, TypedVectorQueryProcessor>
    for InitializedReprojection
{
    fn query_processor(&self) -> Result<TypedVectorQueryProcessor> {
        let state = self.state;
        match self.vector_sources[0].query_processor()? {
            // TODO: use macro for that
            TypedVectorQueryProcessor::Data(source) => Ok(TypedVectorQueryProcessor::Data(
                MapQueryProcessor::new(source, move |query| {
                    query_rewrite_fn(query, state.0, state.1)
                })
                .boxed(),
            )),
            TypedVectorQueryProcessor::MultiPoint(source) => {
                Ok(TypedVectorQueryProcessor::MultiPoint(
                    ReprojectionProcessor::new(source, self.state.0, self.state.1).boxed(),
                ))
            }
            TypedVectorQueryProcessor::MultiLineString(source) => {
                Ok(TypedVectorQueryProcessor::MultiLineString(
                    ReprojectionProcessor::new(source, self.state.0, self.state.1).boxed(),
                ))
            }
            TypedVectorQueryProcessor::MultiPolygon(source) => {
                Ok(TypedVectorQueryProcessor::MultiPolygon(
                    ReprojectionProcessor::new(source, self.state.0, self.state.1).boxed(),
                ))
            }
        }
    }
}

struct ReprojectionProcessor<Q, G> {
    source: Q,
    from: SpatialReference,
    to: SpatialReference,
    geom_type: PhantomData<G>,
}

impl<Q, G> ReprojectionProcessor<Q, G> {
    pub fn new(source: Q, from: SpatialReference, to: SpatialReference) -> Self {
        Self {
            source,
            from,
            to,
            geom_type: PhantomData,
        }
    }
}

/// this method performs the reverse transformation of a query rectangle
fn query_rewrite_fn(
    query: QueryRectangle,
    from: SpatialReference,
    to: SpatialReference,
) -> QueryRectangle {
    let projector = CoordinateProjector::from_known_srs(to, from).unwrap();
    // TODO: change the resolution...
    QueryRectangle {
        bbox: query.bbox.reproject(&projector).unwrap(),
        ..query
    }
}

impl<Q, G> VectorQueryProcessor for ReprojectionProcessor<Q, G>
where
    Q: VectorQueryProcessor<VectorType = G>,
    G: Reproject<CoordinateProjector> + Sync + Send,
{
    type VectorType = G::Out;

    fn vector_query<'a>(
        &'a self,
        query: QueryRectangle,
        ctx: &'a dyn QueryContext,
    ) -> futures::stream::BoxStream<'a, Result<Self::VectorType>> {
        let rewritten_query = query_rewrite_fn(query, self.from, self.to);

        Box::pin(
            self.source
                .vector_query(rewritten_query, ctx)
                .map_ok(move |collection| {
                    collection
                        .reproject(
                            CoordinateProjector::from_known_srs(self.from, self.to)
                                .unwrap()
                                .as_ref(),
                        )
                        .unwrap()
                }),
        )
    }
}

#[cfg(test)]
mod tests {

    use geoengine_datatypes::{
        collections::{MultiLineStringCollection, MultiPointCollection, MultiPolygonCollection},
        primitives::{
            BoundingBox2D, MultiLineString, MultiPoint, MultiPolygon, SpatialResolution,
            TimeInterval,
        },
        spatial_reference::SpatialReferenceAuthority,
        util::well_known_data::{
            COLOGNE_EPSG_4326, COLOGNE_EPSG_900_913, HAMBURG_EPSG_4326, HAMBURG_EPSG_900_913,
            MARBURG_EPSG_4326, MARBURG_EPSG_900_913,
        },
    };

    use crate::engine::{MockExecutionContext, MockQueryContext, QueryProcessor};
    use crate::mock::MockFeatureCollectionSource;
    use futures::StreamExt;

    use super::*;

    #[tokio::test]
    async fn multi_point() -> Result<()> {
        let points = MultiPointCollection::from_data(
            MultiPoint::many(vec![
                MARBURG_EPSG_4326,
                COLOGNE_EPSG_4326,
                HAMBURG_EPSG_4326,
            ])
            .unwrap(),
            vec![TimeInterval::new_unchecked(0, 1); 3],
            Default::default(),
        )?;

        let projected_points = MultiPointCollection::from_data(
            MultiPoint::many(vec![
                MARBURG_EPSG_900_913,
                COLOGNE_EPSG_900_913,
                HAMBURG_EPSG_900_913,
            ])
            .unwrap(),
            vec![TimeInterval::new_unchecked(0, 1); 3],
            Default::default(),
        )?;

        let point_source = MockFeatureCollectionSource::single(points.clone()).boxed();

        let target_spatial_reference =
            SpatialReference::new(SpatialReferenceAuthority::Epsg, 900913);

        let initialized_operator = Reprojection {
            vector_sources: vec![point_source],
            raster_sources: vec![],
            params: ReprojectionParams {
                target_spatial_reference,
            },
        }
        .boxed()
        .initialize(&MockExecutionContext::default())?;

        let query_processor = initialized_operator.query_processor()?;

        let query_processor = query_processor.multi_point().unwrap();

        let query_rectangle = QueryRectangle {
            bbox: BoundingBox2D::new(
                (COLOGNE_EPSG_4326.x, MARBURG_EPSG_4326.y).into(),
                (MARBURG_EPSG_4326.x, HAMBURG_EPSG_4326.y).into(),
            )
            .unwrap(),
            time_interval: TimeInterval::default(),
            spatial_resolution: SpatialResolution::zero_point_one(),
        };
        let ctx = MockQueryContext::new(usize::MAX);

        let query = query_processor.query(query_rectangle, &ctx);

        let result = query
            .map(Result::unwrap)
            .collect::<Vec<MultiPointCollection>>()
            .await;

        assert_eq!(result.len(), 1);

        assert_eq!(result[0], projected_points);

        Ok(())
    }

    #[tokio::test]
    async fn multi_lines() -> Result<()> {
        let lines = MultiLineStringCollection::from_data(
            vec![MultiLineString::new(vec![vec![
                MARBURG_EPSG_4326,
                COLOGNE_EPSG_4326,
                HAMBURG_EPSG_4326,
            ]])
            .unwrap()],
            vec![TimeInterval::new_unchecked(0, 1); 1],
            Default::default(),
        )?;

        let projected_lines = MultiLineStringCollection::from_data(
            vec![MultiLineString::new(vec![vec![
                MARBURG_EPSG_900_913,
                COLOGNE_EPSG_900_913,
                HAMBURG_EPSG_900_913,
            ]])
            .unwrap()],
            vec![TimeInterval::new_unchecked(0, 1); 1],
            Default::default(),
        )?;

        let lines_source = MockFeatureCollectionSource::single(lines.clone()).boxed();

        let target_spatial_reference =
            SpatialReference::new(SpatialReferenceAuthority::Epsg, 900913);

        let initialized_operator = Reprojection {
            vector_sources: vec![lines_source],
            raster_sources: vec![],
            params: ReprojectionParams {
                target_spatial_reference,
            },
        }
        .boxed()
        .initialize(&MockExecutionContext::default())?;

        let query_processor = initialized_operator.query_processor()?;

        let query_processor = query_processor.multi_line_string().unwrap();

        let query_rectangle = QueryRectangle {
            bbox: BoundingBox2D::new(
                (COLOGNE_EPSG_4326.x, MARBURG_EPSG_4326.y).into(),
                (MARBURG_EPSG_4326.x, HAMBURG_EPSG_4326.y).into(),
            )
            .unwrap(),
            time_interval: TimeInterval::default(),
            spatial_resolution: SpatialResolution::zero_point_one(),
        };
        let ctx = MockQueryContext::new(usize::MAX);

        let query = query_processor.query(query_rectangle, &ctx);

        let result = query
            .map(Result::unwrap)
            .collect::<Vec<MultiLineStringCollection>>()
            .await;

        assert_eq!(result.len(), 1);

        assert_eq!(result[0], projected_lines);

        Ok(())
    }

    #[tokio::test]
    async fn multi_polygons() -> Result<()> {
        let polygons = MultiPolygonCollection::from_data(
            vec![MultiPolygon::new(vec![vec![vec![
                MARBURG_EPSG_4326,
                COLOGNE_EPSG_4326,
                HAMBURG_EPSG_4326,
                MARBURG_EPSG_4326,
            ]]])
            .unwrap()],
            vec![TimeInterval::new_unchecked(0, 1); 1],
            Default::default(),
        )?;

        let projected_polygons = MultiPolygonCollection::from_data(
            vec![MultiPolygon::new(vec![vec![vec![
                MARBURG_EPSG_900_913,
                COLOGNE_EPSG_900_913,
                HAMBURG_EPSG_900_913,
                MARBURG_EPSG_900_913,
            ]]])
            .unwrap()],
            vec![TimeInterval::new_unchecked(0, 1); 1],
            Default::default(),
        )?;

        let polygon_source = MockFeatureCollectionSource::single(polygons.clone()).boxed();

        let target_spatial_reference =
            SpatialReference::new(SpatialReferenceAuthority::Epsg, 900913);

        let initialized_operator = Reprojection {
            vector_sources: vec![polygon_source],
            raster_sources: vec![],
            params: ReprojectionParams {
                target_spatial_reference,
            },
        }
        .boxed()
        .initialize(&MockExecutionContext::default())?;

        let query_processor = initialized_operator.query_processor()?;

        let query_processor = query_processor.multi_polygon().unwrap();

        let query_rectangle = QueryRectangle {
            bbox: BoundingBox2D::new(
                (COLOGNE_EPSG_4326.x, MARBURG_EPSG_4326.y).into(),
                (MARBURG_EPSG_4326.x, HAMBURG_EPSG_4326.y).into(),
            )
            .unwrap(),
            time_interval: TimeInterval::default(),
            spatial_resolution: SpatialResolution::zero_point_one(),
        };
        let ctx = MockQueryContext::new(usize::MAX);

        let query = query_processor.query(query_rectangle, &ctx);

        let result = query
            .map(Result::unwrap)
            .collect::<Vec<MultiPolygonCollection>>()
            .await;

        assert_eq!(result.len(), 1);

        assert_eq!(result[0], projected_polygons);

        Ok(())
    }
}
