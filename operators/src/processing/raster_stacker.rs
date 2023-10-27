use crate::adapters::RasterStackerAdapter;
use crate::engine::{
    CanonicOperatorName, ExecutionContext, InitializedRasterOperator, InitializedSources,
    MultipleRasterSources, Operator, OperatorName, QueryContext, RasterOperator,
    RasterQueryProcessor, RasterResultDescriptor, TypedRasterQueryProcessor, WorkflowOperatorPath,
};
use crate::util::Result;
use async_trait::async_trait;
use futures::future::join_all;
use futures::stream::BoxStream;
use geoengine_datatypes::primitives::{
    partitions_extent, time_interval_extent, BandRange, BandSelection, RasterQueryRectangle,
    SpatialResolution,
};
use geoengine_datatypes::raster::{DynamicRasterDataType, Pixel, RasterTile2D};
use serde::{Deserialize, Serialize};

// TODO: IF this operator shall perform spatio-temporal alignment automatically: specify the alignment strategy here
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RasterStackerParams {}

/// This `QueryProcessor` stacks all of it's inputs into a single raster time-series.
/// It does so by querying all of it's inputs outputting them by band, space and then time.
///
/// All inputs must have the same data type and spatial reference.
// TODO: spatio-temporal alignment(?) or do that beforehand?
//     if we explicitly align beforehand using custom operators we have the problem that we need to hardcode the alignment params(?) and if the dataset changes the workflow no longer works
//      we have no way of aligning indepentently of each other before putting them into the `RasterStacker`` as we cannot access other operators in the workflow
pub type RasterStacker = Operator<RasterStackerParams, MultipleRasterSources>;

impl OperatorName for RasterStacker {
    const TYPE_NAME: &'static str = "RasterStacker";
}

#[typetag::serde]
#[async_trait]
impl RasterOperator for RasterStacker {
    async fn _initialize(
        self: Box<Self>,
        path: WorkflowOperatorPath,
        context: &dyn ExecutionContext,
    ) -> Result<Box<dyn InitializedRasterOperator>> {
        let name = CanonicOperatorName::from(&self);

        // TODO: ensure at least two inputs

        // TODO: verify all inputs have same data type and spatial reference

        // TODO: add sparse fill adapter on top of sources? Or do we guarantee gap-free already?

        // TODO: inject operators ontop of the sources to align them spatio-temporally

        let raster_sources = self
            .sources
            .initialize_sources(path, context)
            .await?
            .rasters;

        let in_descriptors = raster_sources
            .iter()
            .map(InitializedRasterOperator::result_descriptor)
            .collect::<Vec<_>>();

        let time = time_interval_extent(in_descriptors.iter().map(|d| d.time));
        let bbox = partitions_extent(in_descriptors.iter().map(|d| d.bbox));

        let resolution = in_descriptors
            .iter()
            .map(|d| d.resolution)
            .reduce(|a, b| match (a, b) {
                (Some(a), Some(b)) => {
                    Some(SpatialResolution::new_unchecked(a.x.min(b.x), a.y.min(b.y)))
                }
                _ => None,
            })
            .flatten();

        let bands_per_source = in_descriptors
            .iter()
            .map(|d| d.bands as usize)
            .collect::<Vec<_>>();

        let bands: usize = bands_per_source.iter().sum();

        let result_descriptor = RasterResultDescriptor {
            data_type: in_descriptors[0].data_type,
            spatial_reference: in_descriptors[0].spatial_reference,
            measurement: in_descriptors[0].measurement.clone(),
            time,
            bbox,
            resolution,
            bands: bands as i32,
        };

        Ok(Box::new(InitializedRasterStacker {
            name,
            result_descriptor,
            raster_sources,
            bands_per_source,
        }))
    }

    span_fn!(RasterStacker);
}

pub struct InitializedRasterStacker {
    name: CanonicOperatorName,
    result_descriptor: RasterResultDescriptor,
    raster_sources: Vec<Box<dyn InitializedRasterOperator>>,
    bands_per_source: Vec<usize>,
}

impl InitializedRasterOperator for InitializedRasterStacker {
    fn result_descriptor(&self) -> &RasterResultDescriptor {
        &self.result_descriptor
    }

    fn query_processor(&self) -> Result<TypedRasterQueryProcessor> {
        let typed_raster_processors = self
            .raster_sources
            .iter()
            .map(InitializedRasterOperator::query_processor)
            .collect::<Result<Vec<_>>>()?;

        // unpack all processors
        let datatype = typed_raster_processors[0].raster_data_type();

        let bands_per_source = self.bands_per_source.clone();

        // TODO: use a macro to unpack all the input processor to the same datatype?
        Ok(match datatype {
            geoengine_datatypes::raster::RasterDataType::U8 => {
                let inputs = typed_raster_processors.into_iter().map(|p| p.get_u8().expect("all inputs should have the same datatype because it was checked in the initialization of the operator")).collect();
                let p = RasterStackerProcessor::new(inputs, bands_per_source);
                TypedRasterQueryProcessor::U8(Box::new(p))
            }
            geoengine_datatypes::raster::RasterDataType::U16 => {
                let inputs = typed_raster_processors.into_iter().map(|p| p.get_u16().expect("all inputs should have the same datatype because it was checked in the initialization of the operator")).collect();
                let p = RasterStackerProcessor::new(inputs, bands_per_source);
                TypedRasterQueryProcessor::U16(Box::new(p))
            }
            geoengine_datatypes::raster::RasterDataType::U32 => {
                let inputs = typed_raster_processors.into_iter().map(|p| p.get_u32().expect("all inputs should have the same datatype because it was checked in the initialization of the operator")).collect();
                let p = RasterStackerProcessor::new(inputs, bands_per_source);
                TypedRasterQueryProcessor::U32(Box::new(p))
            }
            geoengine_datatypes::raster::RasterDataType::U64 => {
                let inputs = typed_raster_processors.into_iter().map(|p| p.get_u64().expect("all inputs should have the same datatype because it was checked in the initialization of the operator")).collect();
                let p = RasterStackerProcessor::new(inputs, bands_per_source);
                TypedRasterQueryProcessor::U64(Box::new(p))
            }
            geoengine_datatypes::raster::RasterDataType::I8 => {
                let inputs = typed_raster_processors.into_iter().map(|p| p.get_i8().expect("all inputs should have the same datatype because it was checked in the initialization of the operator")).collect();
                let p = RasterStackerProcessor::new(inputs, bands_per_source);
                TypedRasterQueryProcessor::I8(Box::new(p))
            }
            geoengine_datatypes::raster::RasterDataType::I16 => {
                let inputs = typed_raster_processors.into_iter().map(|p| p.get_i16().expect("all inputs should have the same datatype because it was checked in the initialization of the operator")).collect();
                let p = RasterStackerProcessor::new(inputs, bands_per_source);
                TypedRasterQueryProcessor::I16(Box::new(p))
            }
            geoengine_datatypes::raster::RasterDataType::I32 => {
                let inputs = typed_raster_processors.into_iter().map(|p| p.get_i32().expect("all inputs should have the same datatype because it was checked in the initialization of the operator")).collect();
                let p = RasterStackerProcessor::new(inputs, bands_per_source);
                TypedRasterQueryProcessor::I32(Box::new(p))
            }
            geoengine_datatypes::raster::RasterDataType::I64 => {
                let inputs = typed_raster_processors.into_iter().map(|p| p.get_i64().expect("all inputs should have the same datatype because it was checked in the initialization of the operator")).collect();
                let p = RasterStackerProcessor::new(inputs, bands_per_source);
                TypedRasterQueryProcessor::I64(Box::new(p))
            }
            geoengine_datatypes::raster::RasterDataType::F32 => {
                let inputs = typed_raster_processors.into_iter().map(|p| p.get_f32().expect("all inputs should have the same datatype because it was checked in the initialization of the operator")).collect();
                let p = RasterStackerProcessor::new(inputs, bands_per_source);
                TypedRasterQueryProcessor::F32(Box::new(p))
            }
            geoengine_datatypes::raster::RasterDataType::F64 => {
                let inputs = typed_raster_processors.into_iter().map(|p| p.get_f64().expect("all inputs should have the same datatype because it was checked in the initialization of the operator")).collect();
                let p = RasterStackerProcessor::new(inputs, bands_per_source);
                TypedRasterQueryProcessor::F64(Box::new(p))
            }
        })
    }

    fn canonic_name(&self) -> CanonicOperatorName {
        self.name.clone()
    }
}

pub(crate) struct RasterStackerProcessor<T> {
    sources: Vec<Box<dyn RasterQueryProcessor<RasterType = T>>>,
    bands_per_source: Vec<usize>,
}

impl<T> RasterStackerProcessor<T> {
    pub fn new(
        sources: Vec<Box<dyn RasterQueryProcessor<RasterType = T>>>,
        bands_per_source: Vec<usize>,
    ) -> Self {
        Self {
            sources,
            bands_per_source,
        }
    }
}

/// compute the bands in the input source from the bands in a query that uses multiple sources
// TODO: adjust this to individual band selection once the query rectangle supports this.
fn map_query_bands_to_source_bands(
    query_bands: BandSelection,
    bands_per_source: &[usize],
    source_index: usize,
) -> Option<BandSelection> {
    let source_start = bands_per_source.iter().take(source_index).sum();
    let source_bands = bands_per_source[source_index];
    let source_end = source_start + source_bands;

    Some(match query_bands {
        BandSelection::Single(b) if b >= source_start && b < source_end => {
            BandSelection::Single(b - source_start)
        }
        BandSelection::Range(BandRange { start, end }) => {
            if start >= source_end || end < source_start {
                return None; // source is not included in query
            }

            let input_start = if start < source_start {
                0
            } else {
                start - source_start
            };

            let input_end = if end >= source_end {
                source_bands
            } else {
                end - source_start
            };

            BandSelection::Range(BandRange {
                start: input_start,
                end: input_end,
            })
        }
        BandSelection::Single(_) => return None, // source is not included in query
    })
}

#[async_trait]
impl<T> RasterQueryProcessor for RasterStackerProcessor<T>
where
    T: Pixel,
{
    type RasterType = T;
    async fn raster_query<'a>(
        &'a self,
        query: RasterQueryRectangle,
        ctx: &'a dyn QueryContext,
    ) -> Result<BoxStream<'a, Result<RasterTile2D<T>>>> {
        let mut source_stream_futures = vec![];
        let mut selected_bands_per_source = vec![];

        for (idx, source) in self.sources.iter().enumerate() {
            let Some(bands) =
                map_query_bands_to_source_bands(query.bands, &self.bands_per_source, idx)
            else {
                continue;
            };

            let mut source_query = query;
            source_query.bands = bands;
            source_stream_futures.push(async move { source.raster_query(source_query, ctx).await });
            selected_bands_per_source.push(bands.count());
        }

        let source_streams = join_all(source_stream_futures)
            .await
            .into_iter()
            .collect::<Result<Vec<_>>>()?;

        let output = RasterStackerAdapter::new(source_streams, selected_bands_per_source);

        Ok(Box::pin(output))
    }
}

#[cfg(test)]
mod tests {
    use futures::StreamExt;
    use geoengine_datatypes::{
        primitives::{BandSelection, CacheHint, Measurement, SpatialPartition2D, TimeInterval},
        raster::{Grid, GridShape, RasterDataType, TilesEqualIgnoringCacheHint},
        spatial_reference::SpatialReference,
        util::test::TestDefault,
    };

    use crate::{
        engine::{MockExecutionContext, MockQueryContext},
        mock::{MockRasterSource, MockRasterSourceParams},
    };

    use super::*;

    #[test]
    fn it_maps_query_bands_to_source_bands() {
        assert_eq!(
            map_query_bands_to_source_bands(BandSelection::Single(0), &[2, 1], 0),
            Some(BandSelection::Single(0))
        );
        assert_eq!(
            map_query_bands_to_source_bands(BandSelection::Single(0), &[2, 1], 1),
            None
        );
        assert_eq!(
            map_query_bands_to_source_bands(BandSelection::Single(2), &[2, 1], 1),
            Some(BandSelection::Single(0))
        );

        assert_eq!(
            map_query_bands_to_source_bands(BandSelection::new_range(0, 1), &[2, 1], 0),
            Some(BandSelection::new_range(0, 1))
        );
        assert_eq!(
            map_query_bands_to_source_bands(BandSelection::new_range(0, 3), &[2, 1], 1),
            Some(BandSelection::new_range(0, 1))
        );
        assert_eq!(
            map_query_bands_to_source_bands(BandSelection::new_range(0, 3), &[2, 2, 1], 1),
            Some(BandSelection::new_range(0, 1))
        );
        assert_eq!(
            map_query_bands_to_source_bands(BandSelection::new_range(0, 4), &[2, 2, 1], 1),
            Some(BandSelection::new_range(0, 2))
        );
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)]
    async fn it_stacks() {
        let data: Vec<RasterTile2D<u8>> = vec![
            RasterTile2D {
                time: TimeInterval::new_unchecked(0, 5),
                tile_position: [-1, 0].into(),
                band: 0,
                global_geo_transform: TestDefault::test_default(),
                grid_array: Grid::new([2, 2].into(), vec![0, 1, 2, 3]).unwrap().into(),
                properties: Default::default(),
                cache_hint: CacheHint::default(),
            },
            RasterTile2D {
                time: TimeInterval::new_unchecked(0, 5),
                tile_position: [-1, 1].into(),
                band: 0,
                global_geo_transform: TestDefault::test_default(),
                grid_array: Grid::new([2, 2].into(), vec![4, 5, 6, 7]).unwrap().into(),
                properties: Default::default(),
                cache_hint: CacheHint::default(),
            },
            RasterTile2D {
                time: TimeInterval::new_unchecked(5, 10),
                tile_position: [-1, 0].into(),
                band: 0,
                global_geo_transform: TestDefault::test_default(),
                grid_array: Grid::new([2, 2].into(), vec![8, 9, 10, 11]).unwrap().into(),
                properties: Default::default(),
                cache_hint: CacheHint::default(),
            },
            RasterTile2D {
                time: TimeInterval::new_unchecked(5, 10),
                tile_position: [-1, 1].into(),
                band: 0,
                global_geo_transform: TestDefault::test_default(),
                grid_array: Grid::new([2, 2].into(), vec![12, 13, 14, 15])
                    .unwrap()
                    .into(),
                properties: Default::default(),
                cache_hint: CacheHint::default(),
            },
        ];

        let data2: Vec<RasterTile2D<u8>> = vec![
            RasterTile2D {
                time: TimeInterval::new_unchecked(0, 5),
                tile_position: [-1, 0].into(),
                band: 0,
                global_geo_transform: TestDefault::test_default(),
                grid_array: Grid::new([2, 2].into(), vec![16, 17, 18, 19])
                    .unwrap()
                    .into(),
                properties: Default::default(),
                cache_hint: CacheHint::default(),
            },
            RasterTile2D {
                time: TimeInterval::new_unchecked(0, 5),
                tile_position: [-1, 1].into(),
                band: 0,
                global_geo_transform: TestDefault::test_default(),
                grid_array: Grid::new([2, 2].into(), vec![20, 21, 22, 23])
                    .unwrap()
                    .into(),
                properties: Default::default(),
                cache_hint: CacheHint::default(),
            },
            RasterTile2D {
                time: TimeInterval::new_unchecked(5, 10),
                tile_position: [-1, 0].into(),
                band: 0,
                global_geo_transform: TestDefault::test_default(),
                grid_array: Grid::new([2, 2].into(), vec![24, 25, 26, 27])
                    .unwrap()
                    .into(),
                properties: Default::default(),
                cache_hint: CacheHint::default(),
            },
            RasterTile2D {
                time: TimeInterval::new_unchecked(5, 10),
                tile_position: [-1, 1].into(),
                band: 0,
                global_geo_transform: TestDefault::test_default(),
                grid_array: Grid::new([2, 2].into(), vec![28, 29, 30, 31])
                    .unwrap()
                    .into(),
                properties: Default::default(),
                cache_hint: CacheHint::default(),
            },
        ];

        let mrs1 = MockRasterSource {
            params: MockRasterSourceParams {
                data: data.clone(),
                result_descriptor: RasterResultDescriptor {
                    data_type: RasterDataType::U8,
                    spatial_reference: SpatialReference::epsg_4326().into(),
                    measurement: Measurement::Unitless,
                    time: None,
                    bbox: None,
                    resolution: None,
                    bands: 1,
                },
            },
        }
        .boxed();

        let mrs2 = MockRasterSource {
            params: MockRasterSourceParams {
                data: data2.clone(),
                result_descriptor: RasterResultDescriptor {
                    data_type: RasterDataType::U8,
                    spatial_reference: SpatialReference::epsg_4326().into(),
                    measurement: Measurement::Unitless,
                    time: None,
                    bbox: None,
                    resolution: None,
                    bands: 1,
                },
            },
        }
        .boxed();

        let stacker = RasterStacker {
            params: RasterStackerParams {},
            sources: MultipleRasterSources {
                rasters: vec![mrs1, mrs2],
            },
        }
        .boxed();

        let mut exe_ctx = MockExecutionContext::test_default();
        exe_ctx.tiling_specification.tile_size_in_pixels = GridShape {
            shape_array: [2, 2],
        };

        let query_rect = RasterQueryRectangle {
            spatial_bounds: SpatialPartition2D::new_unchecked((0., 1.).into(), (3., 0.).into()),
            time_interval: TimeInterval::new_unchecked(0, 10),
            spatial_resolution: SpatialResolution::one(),
            bands: BandSelection::new_range(0, 2),
        };

        let query_ctx = MockQueryContext::test_default();

        let op = stacker
            .initialize(WorkflowOperatorPath::initialize_root(), &exe_ctx)
            .await
            .unwrap();

        let qp = op.query_processor().unwrap().get_u8().unwrap();

        let result = qp
            .raster_query(query_rect, &query_ctx)
            .await
            .unwrap()
            .collect::<Vec<_>>()
            .await;
        let result = result.into_iter().collect::<Result<Vec<_>>>().unwrap();

        let expected: Vec<_> = data
            .into_iter()
            .zip(data2.into_iter().map(|mut tile| {
                tile.band = 1;
                tile
            }))
            .flat_map(|(a, b)| vec![a.clone(), b.clone()])
            .collect();

        assert!(expected.tiles_equal_ignoring_cache_hint(&result));
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)]
    async fn it_stacks_stacks() {
        let data: Vec<RasterTile2D<u8>> = vec![
            RasterTile2D {
                time: TimeInterval::new_unchecked(0, 5),
                tile_position: [-1, 0].into(),
                band: 0,
                global_geo_transform: TestDefault::test_default(),
                grid_array: Grid::new([2, 2].into(), vec![0, 1, 2, 3]).unwrap().into(),
                properties: Default::default(),
                cache_hint: CacheHint::default(),
            },
            RasterTile2D {
                time: TimeInterval::new_unchecked(0, 5),
                tile_position: [-1, 0].into(),
                band: 1,
                global_geo_transform: TestDefault::test_default(),
                grid_array: Grid::new([2, 2].into(), vec![3, 2, 1, 0]).unwrap().into(),
                properties: Default::default(),
                cache_hint: CacheHint::default(),
            },
            RasterTile2D {
                time: TimeInterval::new_unchecked(0, 5),
                tile_position: [-1, 1].into(),
                band: 0,
                global_geo_transform: TestDefault::test_default(),
                grid_array: Grid::new([2, 2].into(), vec![4, 5, 6, 7]).unwrap().into(),
                properties: Default::default(),
                cache_hint: CacheHint::default(),
            },
            RasterTile2D {
                time: TimeInterval::new_unchecked(0, 5),
                tile_position: [-1, 1].into(),
                band: 1,
                global_geo_transform: TestDefault::test_default(),
                grid_array: Grid::new([2, 2].into(), vec![7, 6, 5, 4]).unwrap().into(),
                properties: Default::default(),
                cache_hint: CacheHint::default(),
            },
            RasterTile2D {
                time: TimeInterval::new_unchecked(5, 10),
                tile_position: [-1, 0].into(),
                band: 0,
                global_geo_transform: TestDefault::test_default(),
                grid_array: Grid::new([2, 2].into(), vec![8, 9, 10, 11]).unwrap().into(),
                properties: Default::default(),
                cache_hint: CacheHint::default(),
            },
            RasterTile2D {
                time: TimeInterval::new_unchecked(5, 10),
                tile_position: [-1, 0].into(),
                band: 1,
                global_geo_transform: TestDefault::test_default(),
                grid_array: Grid::new([2, 2].into(), vec![11, 10, 9, 8]).unwrap().into(),
                properties: Default::default(),
                cache_hint: CacheHint::default(),
            },
            RasterTile2D {
                time: TimeInterval::new_unchecked(5, 10),
                tile_position: [-1, 1].into(),
                band: 0,
                global_geo_transform: TestDefault::test_default(),
                grid_array: Grid::new([2, 2].into(), vec![12, 13, 14, 15])
                    .unwrap()
                    .into(),
                properties: Default::default(),
                cache_hint: CacheHint::default(),
            },
            RasterTile2D {
                time: TimeInterval::new_unchecked(5, 10),
                tile_position: [-1, 1].into(),
                band: 1,
                global_geo_transform: TestDefault::test_default(),
                grid_array: Grid::new([2, 2].into(), vec![15, 14, 13, 12])
                    .unwrap()
                    .into(),
                properties: Default::default(),
                cache_hint: CacheHint::default(),
            },
        ];

        let data2: Vec<RasterTile2D<u8>> = vec![
            RasterTile2D {
                time: TimeInterval::new_unchecked(0, 5),
                tile_position: [-1, 0].into(),
                band: 0,
                global_geo_transform: TestDefault::test_default(),
                grid_array: Grid::new([2, 2].into(), vec![16, 17, 18, 19])
                    .unwrap()
                    .into(),
                properties: Default::default(),
                cache_hint: CacheHint::default(),
            },
            RasterTile2D {
                time: TimeInterval::new_unchecked(0, 5),
                tile_position: [-1, 0].into(),
                band: 1,
                global_geo_transform: TestDefault::test_default(),
                grid_array: Grid::new([2, 2].into(), vec![19, 18, 17, 16])
                    .unwrap()
                    .into(),
                properties: Default::default(),
                cache_hint: CacheHint::default(),
            },
            RasterTile2D {
                time: TimeInterval::new_unchecked(0, 5),
                tile_position: [-1, 1].into(),
                band: 0,
                global_geo_transform: TestDefault::test_default(),
                grid_array: Grid::new([2, 2].into(), vec![20, 21, 22, 23])
                    .unwrap()
                    .into(),
                properties: Default::default(),
                cache_hint: CacheHint::default(),
            },
            RasterTile2D {
                time: TimeInterval::new_unchecked(0, 5),
                tile_position: [-1, 1].into(),
                band: 1,
                global_geo_transform: TestDefault::test_default(),
                grid_array: Grid::new([2, 2].into(), vec![32, 22, 21, 20])
                    .unwrap()
                    .into(),
                properties: Default::default(),
                cache_hint: CacheHint::default(),
            },
            RasterTile2D {
                time: TimeInterval::new_unchecked(5, 10),
                tile_position: [-1, 0].into(),
                band: 0,
                global_geo_transform: TestDefault::test_default(),
                grid_array: Grid::new([2, 2].into(), vec![24, 25, 26, 27])
                    .unwrap()
                    .into(),
                properties: Default::default(),
                cache_hint: CacheHint::default(),
            },
            RasterTile2D {
                time: TimeInterval::new_unchecked(5, 10),
                tile_position: [-1, 0].into(),
                band: 1,
                global_geo_transform: TestDefault::test_default(),
                grid_array: Grid::new([2, 2].into(), vec![27, 26, 25, 24])
                    .unwrap()
                    .into(),
                properties: Default::default(),
                cache_hint: CacheHint::default(),
            },
            RasterTile2D {
                time: TimeInterval::new_unchecked(5, 10),
                tile_position: [-1, 1].into(),
                band: 0,
                global_geo_transform: TestDefault::test_default(),
                grid_array: Grid::new([2, 2].into(), vec![28, 29, 30, 31])
                    .unwrap()
                    .into(),
                properties: Default::default(),
                cache_hint: CacheHint::default(),
            },
            RasterTile2D {
                time: TimeInterval::new_unchecked(5, 10),
                tile_position: [-1, 1].into(),
                band: 1,
                global_geo_transform: TestDefault::test_default(),
                grid_array: Grid::new([2, 2].into(), vec![31, 30, 39, 28])
                    .unwrap()
                    .into(),
                properties: Default::default(),
                cache_hint: CacheHint::default(),
            },
        ];

        let mrs1 = MockRasterSource {
            params: MockRasterSourceParams {
                data: data.clone(),
                result_descriptor: RasterResultDescriptor {
                    data_type: RasterDataType::U8,
                    spatial_reference: SpatialReference::epsg_4326().into(),
                    measurement: Measurement::Unitless,
                    time: None,
                    bbox: None,
                    resolution: None,
                    bands: 2,
                },
            },
        }
        .boxed();

        let mrs2 = MockRasterSource {
            params: MockRasterSourceParams {
                data: data2.clone(),
                result_descriptor: RasterResultDescriptor {
                    data_type: RasterDataType::U8,
                    spatial_reference: SpatialReference::epsg_4326().into(),
                    measurement: Measurement::Unitless,
                    time: None,
                    bbox: None,
                    resolution: None,
                    bands: 2,
                },
            },
        }
        .boxed();

        let stacker = RasterStacker {
            params: RasterStackerParams {},
            sources: MultipleRasterSources {
                rasters: vec![mrs1, mrs2],
            },
        }
        .boxed();

        let mut exe_ctx = MockExecutionContext::test_default();
        exe_ctx.tiling_specification.tile_size_in_pixels = GridShape {
            shape_array: [2, 2],
        };

        let query_rect = RasterQueryRectangle {
            spatial_bounds: SpatialPartition2D::new_unchecked((0., 1.).into(), (3., 0.).into()),
            time_interval: TimeInterval::new_unchecked(0, 10),
            spatial_resolution: SpatialResolution::one(),
            bands: BandSelection::new_range(0, 4),
        };

        let query_ctx = MockQueryContext::test_default();

        let op = stacker
            .initialize(WorkflowOperatorPath::initialize_root(), &exe_ctx)
            .await
            .unwrap();

        let qp = op.query_processor().unwrap().get_u8().unwrap();

        let result = qp
            .raster_query(query_rect, &query_ctx)
            .await
            .unwrap()
            .collect::<Vec<_>>()
            .await;
        let result = result.into_iter().collect::<Result<Vec<_>>>().unwrap();

        let expected: Vec<_> = data
            .chunks(2)
            .zip(
                data2
                    .into_iter()
                    .map(|mut tile| {
                        tile.band += 2;
                        tile
                    })
                    .collect::<Vec<_>>()
                    .chunks(2),
            )
            .flat_map(|(chunk1, chunk2)| chunk1.iter().chain(chunk2.iter()))
            .cloned()
            .collect();

        assert!(expected.tiles_equal_ignoring_cache_hint(&result));
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)]
    async fn it_selects_band_from_stack() {
        let data: Vec<RasterTile2D<u8>> = vec![
            RasterTile2D {
                time: TimeInterval::new_unchecked(0, 5),
                tile_position: [-1, 0].into(),
                band: 0,
                global_geo_transform: TestDefault::test_default(),
                grid_array: Grid::new([2, 2].into(), vec![0, 1, 2, 3]).unwrap().into(),
                properties: Default::default(),
                cache_hint: CacheHint::default(),
            },
            RasterTile2D {
                time: TimeInterval::new_unchecked(0, 5),
                tile_position: [-1, 1].into(),
                band: 0,
                global_geo_transform: TestDefault::test_default(),
                grid_array: Grid::new([2, 2].into(), vec![4, 5, 6, 7]).unwrap().into(),
                properties: Default::default(),
                cache_hint: CacheHint::default(),
            },
            RasterTile2D {
                time: TimeInterval::new_unchecked(5, 10),
                tile_position: [-1, 0].into(),
                band: 0,
                global_geo_transform: TestDefault::test_default(),
                grid_array: Grid::new([2, 2].into(), vec![8, 9, 10, 11]).unwrap().into(),
                properties: Default::default(),
                cache_hint: CacheHint::default(),
            },
            RasterTile2D {
                time: TimeInterval::new_unchecked(5, 10),
                tile_position: [-1, 1].into(),
                band: 0,
                global_geo_transform: TestDefault::test_default(),
                grid_array: Grid::new([2, 2].into(), vec![12, 13, 14, 15])
                    .unwrap()
                    .into(),
                properties: Default::default(),
                cache_hint: CacheHint::default(),
            },
        ];

        let data2: Vec<RasterTile2D<u8>> = vec![
            RasterTile2D {
                time: TimeInterval::new_unchecked(0, 5),
                tile_position: [-1, 0].into(),
                band: 0,
                global_geo_transform: TestDefault::test_default(),
                grid_array: Grid::new([2, 2].into(), vec![16, 17, 18, 19])
                    .unwrap()
                    .into(),
                properties: Default::default(),
                cache_hint: CacheHint::default(),
            },
            RasterTile2D {
                time: TimeInterval::new_unchecked(0, 5),
                tile_position: [-1, 1].into(),
                band: 0,
                global_geo_transform: TestDefault::test_default(),
                grid_array: Grid::new([2, 2].into(), vec![20, 21, 22, 23])
                    .unwrap()
                    .into(),
                properties: Default::default(),
                cache_hint: CacheHint::default(),
            },
            RasterTile2D {
                time: TimeInterval::new_unchecked(5, 10),
                tile_position: [-1, 0].into(),
                band: 0,
                global_geo_transform: TestDefault::test_default(),
                grid_array: Grid::new([2, 2].into(), vec![24, 25, 26, 27])
                    .unwrap()
                    .into(),
                properties: Default::default(),
                cache_hint: CacheHint::default(),
            },
            RasterTile2D {
                time: TimeInterval::new_unchecked(5, 10),
                tile_position: [-1, 1].into(),
                band: 0,
                global_geo_transform: TestDefault::test_default(),
                grid_array: Grid::new([2, 2].into(), vec![28, 29, 30, 31])
                    .unwrap()
                    .into(),
                properties: Default::default(),
                cache_hint: CacheHint::default(),
            },
        ];

        let mrs1 = MockRasterSource {
            params: MockRasterSourceParams {
                data: data.clone(),
                result_descriptor: RasterResultDescriptor {
                    data_type: RasterDataType::U8,
                    spatial_reference: SpatialReference::epsg_4326().into(),
                    measurement: Measurement::Unitless,
                    time: None,
                    bbox: None,
                    resolution: None,
                    bands: 1,
                },
            },
        }
        .boxed();

        let mrs2 = MockRasterSource {
            params: MockRasterSourceParams {
                data: data2.clone(),
                result_descriptor: RasterResultDescriptor {
                    data_type: RasterDataType::U8,
                    spatial_reference: SpatialReference::epsg_4326().into(),
                    measurement: Measurement::Unitless,
                    time: None,
                    bbox: None,
                    resolution: None,
                    bands: 1,
                },
            },
        }
        .boxed();

        let stacker = RasterStacker {
            params: RasterStackerParams {},
            sources: MultipleRasterSources {
                rasters: vec![mrs1, mrs2],
            },
        }
        .boxed();

        let mut exe_ctx = MockExecutionContext::test_default();
        exe_ctx.tiling_specification.tile_size_in_pixels = GridShape {
            shape_array: [2, 2],
        };

        let query_rect = RasterQueryRectangle {
            spatial_bounds: SpatialPartition2D::new_unchecked((0., 1.).into(), (3., 0.).into()),
            time_interval: TimeInterval::new_unchecked(0, 10),
            spatial_resolution: SpatialResolution::one(),
            bands: BandSelection::Single(1),
        };

        let query_ctx = MockQueryContext::test_default();

        let op = stacker
            .initialize(WorkflowOperatorPath::initialize_root(), &exe_ctx)
            .await
            .unwrap();

        let qp = op.query_processor().unwrap().get_u8().unwrap();

        let result = qp
            .raster_query(query_rect, &query_ctx)
            .await
            .unwrap()
            .collect::<Vec<_>>()
            .await;
        let result = result.into_iter().collect::<Result<Vec<_>>>().unwrap();

        assert!(data2.tiles_equal_ignoring_cache_hint(&result));
    }
}
