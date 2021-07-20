use std::path::Path;

use crate::error::Error;
use crate::util::parsing::string_or_string_array;
use crate::{datasets::listing::DatasetListOptions, error::Result};
use crate::{
    datasets::{
        listing::{DatasetListing, DatasetProvider},
        storage::DatasetProviderDefinition,
    },
    error,
    util::user_input::Validated,
};
use async_trait::async_trait;
use futures::future::join_all;
use gdal::DatasetOptions;
use geoengine_datatypes::dataset::{DatasetId, DatasetProviderId, ExternalDatasetId};
use geoengine_operators::engine::TypedResultDescriptor;
use geoengine_operators::source::GdalMetaDataStatic;
use geoengine_operators::util::gdal::{
    gdal_open_dataset_ex, gdal_parameters_from_dataset, raster_descriptor_from_dataset,
};
use geoengine_operators::{
    engine::{
        MetaData, MetaDataProvider, RasterQueryRectangle, RasterResultDescriptor,
        VectorQueryRectangle, VectorResultDescriptor,
    },
    mock::MockDatasetDataSourceLoadingInfo,
    source::{GdalLoadingInfo, OgrSourceDataset},
};
use log::info;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use snafu::ResultExt;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Natur40DataProviderDefinition {
    id: DatasetProviderId,
    name: String,
    base_url: String,
    user: String,
    password: String,
}

#[typetag::serde]
#[async_trait]
impl DatasetProviderDefinition for Natur40DataProviderDefinition {
    async fn initialize(self: Box<Self>) -> crate::error::Result<Box<dyn DatasetProvider>> {
        Ok(Box::new(Natur40DataProvider {
            id: self.id,
            base_url: self.base_url,
            user: self.user,
            password: self.password,
        }))
    }

    fn type_name(&self) -> String {
        "Natur4.0".to_owned()
    }

    fn name(&self) -> String {
        self.name.clone()
    }

    fn id(&self) -> DatasetProviderId {
        self.id
    }
}

pub struct Natur40DataProvider {
    id: DatasetProviderId,
    base_url: String,
    user: String,
    password: String,
}

#[derive(Deserialize, Debug)]
struct RasterDb {
    name: String,
    title: String,
    #[serde(deserialize_with = "string_or_string_array")]
    tags: Vec<String>,
}

impl RasterDb {
    fn url_from_name(base_url: &str, name: &str) -> String {
        format!(
            "WCS:{}/rasterdb/{name}/wcs?VERSION=1.0.0&COVERAGE={name}",
            base_url,
            name = name
        )
    }

    fn url(&self, base_url: &str) -> String {
        Self::url_from_name(base_url, &self.name)
    }
}

#[derive(Deserialize, Debug)]
struct RasterDbs {
    rasterdbs: Vec<RasterDb>,
    #[serde(deserialize_with = "string_or_string_array")]
    tags: Vec<String>,
    session: String,
}

#[async_trait]
impl DatasetProvider for Natur40DataProvider {
    async fn list(&self, _options: Validated<DatasetListOptions>) -> Result<Vec<DatasetListing>> {
        // TODO: query the other dbs as well
        let raster_dbs = self.load_raster_dbs().await?;

        let mut listing = vec![];

        let datasets = raster_dbs
            .rasterdbs
            .iter()
            .map(|db| self.load_dataset(db.url(&self.base_url)));
        let datasets: Vec<Result<gdal::Dataset>> = join_all(datasets).await;

        for (db, dataset) in raster_dbs.rasterdbs.iter().zip(&datasets) {
            if let Ok(dataset) = dataset {
                for band_index in 1..dataset.raster_count() {
                    if let Ok(result_descriptor) =
                        raster_descriptor_from_dataset(&dataset, band_index, None)
                    {
                        // TODO: get label from rasterband

                        listing.push(Ok(DatasetListing {
                            id: DatasetId::External(ExternalDatasetId {
                                provider_id: self.id,
                                dataset_id: format!("{}:{}", db.name.clone(), band_index),
                            }),
                            name: db.title.clone(),
                            description: format!("Band: {}", band_index),
                            tags: db.tags.clone(),
                            source_operator: "GdalSource".to_owned(),
                            result_descriptor: TypedResultDescriptor::Raster(result_descriptor),
                            symbology: None, // TODO: build symbology
                        }));
                    } else {
                        info!(
                            "Could not create restult descriptor for band {} of {}",
                            band_index, db.name
                        );
                    }
                }
            } else {
                info!("Could not open dataset {}", db.name);
            }
        }

        Ok(listing
            .into_iter()
            .filter_map(|d: Result<DatasetListing>| if let Ok(d) = d { Some(d) } else { None })
            .collect())
    }

    async fn load(
        &self,
        // _session: S,
        _dataset: &geoengine_datatypes::dataset::DatasetId,
    ) -> crate::error::Result<crate::datasets::storage::Dataset> {
        Err(error::Error::NotYetImplemented)
    }
}

impl Natur40DataProvider {
    fn auth(&self) -> [String; 2] {
        [
            format!("UserPwd={}:{}", self.user, self.password),
            "HttpAuth=BASIC".to_owned(),
        ]
    }

    async fn load_dataset(&self, db_url: String) -> Result<gdal::Dataset> {
        let auth = self.auth();
        tokio::task::spawn_blocking(move || {
            let dataset = gdal_open_dataset_ex(
                Path::new(&db_url),
                DatasetOptions {
                    open_options: Some(&[&auth[0], &auth[1]]),
                    ..DatasetOptions::default()
                },
            )?;
            Ok(dataset)
        })
        .await
        .context(error::TokioJoin)?
    }

    async fn load_raster_dbs(&self) -> Result<RasterDbs> {
        Client::new()
            .get(format!("{}/rasterdbs.json", self.base_url))
            .basic_auth(&self.user, Some(&self.password))
            .send()
            .await?
            .json()
            .await
            .context(error::Reqwest)
    }
}

#[async_trait]
impl MetaDataProvider<GdalLoadingInfo, RasterResultDescriptor, RasterQueryRectangle>
    for Natur40DataProvider
{
    async fn meta_data(
        &self,
        dataset: &DatasetId,
    ) -> Result<
        Box<dyn MetaData<GdalLoadingInfo, RasterResultDescriptor, RasterQueryRectangle>>,
        geoengine_operators::error::Error,
    > {
        let dataset = dataset
            .external()
            .ok_or(geoengine_operators::error::Error::LoadingInfo {
                source: Box::new(Error::InvalidExternalDatasetId { provider: self.id }),
            })?;
        let split: Vec<_> = dataset.dataset_id.split(':').collect();

        let (db_name, band_index) = match *split.as_slice() {
            [db, band_index] => {
                if let Ok(band_index) = band_index.parse::<isize>() {
                    (db, band_index)
                } else {
                    return Err(geoengine_operators::error::Error::LoadingInfo {
                        source: Box::new(Error::InvalidExternalDatasetId { provider: self.id }),
                    });
                }
            }
            _ => {
                return Err(geoengine_operators::error::Error::LoadingInfo {
                    source: Box::new(Error::InvalidExternalDatasetId { provider: self.id }),
                })
            }
        };

        let db_url = RasterDb::url_from_name(&self.base_url, db_name);
        let dataset = self.load_dataset(db_url.clone()).await.map_err(|e| {
            geoengine_operators::error::Error::LoadingInfo {
                source: Box::new(e),
            }
        })?;

        Ok(Box::new(GdalMetaDataStatic {
            time: None,
            params: gdal_parameters_from_dataset(&dataset, band_index, Path::new(&db_url))?,
            result_descriptor: raster_descriptor_from_dataset(&dataset, band_index, None)?,
        }))
    }
}

#[async_trait]
impl
    MetaDataProvider<MockDatasetDataSourceLoadingInfo, VectorResultDescriptor, VectorQueryRectangle>
    for Natur40DataProvider
{
    async fn meta_data(
        &self,
        _dataset: &DatasetId,
    ) -> Result<
        Box<
            dyn MetaData<
                MockDatasetDataSourceLoadingInfo,
                VectorResultDescriptor,
                VectorQueryRectangle,
            >,
        >,
        geoengine_operators::error::Error,
    > {
        todo!()
    }
}

#[async_trait]
impl MetaDataProvider<OgrSourceDataset, VectorResultDescriptor, VectorQueryRectangle>
    for Natur40DataProvider
{
    async fn meta_data(
        &self,
        _dataset: &DatasetId,
    ) -> Result<
        Box<dyn MetaData<OgrSourceDataset, VectorResultDescriptor, VectorQueryRectangle>>,
        geoengine_operators::error::Error,
    > {
        Err(geoengine_operators::error::Error::NotYetImplemented)
    }
}
