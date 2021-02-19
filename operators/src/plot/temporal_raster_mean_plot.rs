use crate::engine::{
    ExecutionContext, InitializedOperator, InitializedOperatorImpl, InitializedPlotOperator,
    Operator, PlotOperator, PlotQueryProcessor, PlotResultDescriptor, QueryContext, QueryProcessor,
    QueryRectangle, RasterQueryProcessor, TypedPlotQueryProcessor,
};
use crate::error;
use crate::util::math::average_floor;
use crate::util::Result;
use futures::future::BoxFuture;
use futures::stream::BoxStream;
use futures::{FutureExt, StreamExt, TryFutureExt};
use geoengine_datatypes::plots::{AreaLineChart, Plot};
use geoengine_datatypes::primitives::{Measurement, TimeInstance, TimeInterval};
use geoengine_datatypes::raster::{Pixel, RasterTile2D};
use serde::{Deserialize, Serialize};
use snafu::ensure;
use std::collections::BTreeMap;

/// A plot that shows the mean values of rasters over time as an area plot.
pub type TemporalRasterMeanPlot = Operator<TemporalRasterMeanPlotParams>;

/// The parameter spec for `TemporalRasterMeanPlot`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TemporalRasterMeanPlotParams {
    /// Where should the x-axis (time) tick be positioned?
    /// At either time start, time end or in the center.
    time_position: TemporalRasterMeanPlotTimePosition,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TemporalRasterMeanPlotTimePosition {
    Start,
    Center,
    End,
}

#[typetag::serde]
impl PlotOperator for TemporalRasterMeanPlot {
    fn initialize(
        self: Box<Self>,
        context: &dyn ExecutionContext,
    ) -> Result<Box<InitializedPlotOperator>> {
        ensure!(
            self.vector_sources.is_empty(),
            error::InvalidNumberOfVectorInputs {
                expected: 0..1,
                found: self.vector_sources.len()
            }
        );
        ensure!(
            self.raster_sources.len() == 1,
            error::InvalidNumberOfVectorInputs {
                expected: 1..2,
                found: self.raster_sources.len()
            }
        );

        Ok(InitializedTemporalRasterMeanPlot {
            params: self.params,
            result_descriptor: PlotResultDescriptor {},
            raster_sources: self
                .raster_sources
                .into_iter()
                .map(|o| o.initialize(context))
                .collect::<Result<Vec<_>>>()?,
            vector_sources: vec![],
            state: (),
        }
        .boxed())
    }
}

/// The initialization of `TemporalRasterMeanPlot`
pub type InitializedTemporalRasterMeanPlot =
    InitializedOperatorImpl<TemporalRasterMeanPlotParams, PlotResultDescriptor, ()>;

impl InitializedOperator<PlotResultDescriptor, TypedPlotQueryProcessor>
    for InitializedTemporalRasterMeanPlot
{
    fn query_processor(&self) -> Result<TypedPlotQueryProcessor> {
        let input_processor = self.raster_sources[0].query_processor()?;
        let time_position = self.params.time_position;
        let measurement = self.raster_sources[0]
            .result_descriptor()
            .measurement
            .clone();

        let processor = call_on_generic_raster_processor!(input_processor, raster => {
            TemporalRasterMeanPlotQueryProcessor { raster, time_position, measurement }.boxed()
        });

        Ok(TypedPlotQueryProcessor::Json(processor))
    }
}

/// A query processor that calculates the `TemporalRasterMeanPlot` about its input.
pub struct TemporalRasterMeanPlotQueryProcessor<P: Pixel> {
    raster: Box<dyn RasterQueryProcessor<RasterType = P>>,
    time_position: TemporalRasterMeanPlotTimePosition,
    measurement: Measurement,
}

impl<P: Pixel> PlotQueryProcessor for TemporalRasterMeanPlotQueryProcessor<P> {
    // TODO: chart
    type PlotType = serde_json::Value;

    fn plot_query<'a>(
        &'a self,
        query: QueryRectangle,
        ctx: &'a dyn QueryContext,
    ) -> BoxFuture<'a, Result<Self::PlotType>> {
        Self::calculate_means(self.raster.query(query, ctx), self.time_position)
            .and_then(move |means| async move {
                let plot = Self::generate_plot(means, self.measurement.clone())?;
                let plot = plot.to_vega_embeddable(false)?;
                Ok(serde_json::to_value(&plot)?)
            })
            .boxed()
    }
}

impl<P: Pixel> TemporalRasterMeanPlotQueryProcessor<P> {
    async fn calculate_means(
        mut tile_stream: BoxStream<'_, Result<RasterTile2D<P>>>,
        position: TemporalRasterMeanPlotTimePosition,
    ) -> Result<BTreeMap<TimeInstance, MeanCalculator>> {
        let mut means: BTreeMap<TimeInstance, MeanCalculator> = BTreeMap::new();

        while let Some(tile) = tile_stream.next().await {
            let tile = tile?;

            let time = Self::time_interval_projection(tile.time, position);

            let mean = means.entry(time).or_default();
            mean.add(&tile.grid_array.data, tile.grid_array.no_data_value);
        }

        Ok(means)
    }

    #[inline]
    fn time_interval_projection(
        time_interval: TimeInterval,
        time_position: TemporalRasterMeanPlotTimePosition,
    ) -> TimeInstance {
        match time_position {
            TemporalRasterMeanPlotTimePosition::Start => time_interval.start(),
            TemporalRasterMeanPlotTimePosition::Center => TimeInstance::from_millis(average_floor(
                time_interval.start().inner(),
                time_interval.end().inner(),
            )),
            TemporalRasterMeanPlotTimePosition::End => time_interval.end(),
        }
    }

    fn generate_plot(
        means: BTreeMap<TimeInstance, MeanCalculator>,
        measurement: Measurement,
    ) -> Result<AreaLineChart> {
        let mut timestamps = Vec::with_capacity(means.len());
        let mut values = Vec::with_capacity(means.len());

        for (timestamp, mean_calculator) in means {
            timestamps.push(timestamp);
            values.push(mean_calculator.mean());
        }

        AreaLineChart::new(timestamps, values, measurement).map_err(Into::into)
    }
}

struct MeanCalculator {
    mean: f64,
    n: usize,
}

impl Default for MeanCalculator {
    fn default() -> Self {
        Self { mean: 0., n: 0 }
    }
}

impl MeanCalculator {
    #[inline]
    fn add<P: Pixel>(&mut self, values: &[P], no_data: Option<P>) {
        if let Some(no_data) = no_data {
            self.add_with_no_data(values, no_data);
        } else {
            self.add_without_no_data(values);
        }
    }

    #[inline]
    fn add_without_no_data<P: Pixel>(&mut self, values: &[P]) {
        for &value in values {
            self.add_single_value(value);
        }
    }

    #[inline]
    fn add_with_no_data<P: Pixel>(&mut self, values: &[P], no_data: P) {
        for &value in values {
            if value == no_data {
                continue;
            }

            self.add_single_value(value);
        }
    }

    #[inline]
    fn add_single_value<P: Pixel>(&mut self, value: P) {
        let value: f64 = value.as_();

        if value.is_nan() {
            return;
        }

        self.n += 1;
        let delta = value - self.mean;
        self.mean += delta / (self.n as f64);
    }

    #[inline]
    fn mean(&self) -> f64 {
        self.mean
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::engine::{
        MockExecutionContext, MockQueryContext, RasterOperator, RasterResultDescriptor,
    };
    use crate::mock::{MockRasterSource, MockRasterSourceParams};
    use chrono::NaiveDate;
    use geoengine_datatypes::plots::PlotData;
    use geoengine_datatypes::primitives::{
        BoundingBox2D, Measurement, SpatialResolution, TimeInterval,
    };
    use geoengine_datatypes::raster::{Grid2D, RasterDataType, TileInformation};
    use geoengine_datatypes::spatial_reference::SpatialReference;

    #[test]
    fn serialization() {
        let temporal_raster_mean_plot = TemporalRasterMeanPlot {
            params: TemporalRasterMeanPlotParams {
                time_position: TemporalRasterMeanPlotTimePosition::Start,
            },
            raster_sources: vec![],
            vector_sources: vec![],
        };

        let serialized = json!({
            "type": "TemporalRasterMeanPlot",
            "params": {
                "time_position": "start"
            },
            "raster_sources": [],
            "vector_sources": [],
        })
        .to_string();

        let deserialized: TemporalRasterMeanPlot = serde_json::from_str(&serialized).unwrap();

        assert_eq!(deserialized.params, temporal_raster_mean_plot.params);
    }

    #[tokio::test]
    async fn single_raster() {
        let temporal_raster_mean_plot = TemporalRasterMeanPlot {
            params: TemporalRasterMeanPlotParams {
                time_position: TemporalRasterMeanPlotTimePosition::Center,
            },
            raster_sources: vec![generate_mock_raster_source(
                vec![TimeInterval::new(
                    TimeInstance::from(NaiveDate::from_ymd(1990, 1, 1).and_hms(0, 0, 0)),
                    TimeInstance::from(NaiveDate::from_ymd(2000, 1, 1).and_hms(0, 0, 0)),
                )
                .unwrap()],
                vec![vec![1, 2, 3, 4, 5, 6]],
            )],
            vector_sources: vec![],
        };

        let execution_context = MockExecutionContext::default();

        let temporal_raster_mean_plot = temporal_raster_mean_plot
            .boxed()
            .initialize(&execution_context)
            .unwrap();

        let processor = temporal_raster_mean_plot
            .query_processor()
            .unwrap()
            .json()
            .unwrap();

        let result = processor
            .plot_query(
                QueryRectangle {
                    bbox: BoundingBox2D::new((-180., -90.).into(), (180., 90.).into()).unwrap(),
                    time_interval: TimeInterval::default(),
                    spatial_resolution: SpatialResolution::one(),
                },
                &MockQueryContext::new(0),
            )
            .await
            .unwrap();

        assert_eq!(
            result.to_string(),
            json!({
                "vega_string": "{\"$schema\":\"https://vega.github.io/schema/vega-lite/v4.17.0.json\",\"data\":{\"values\":[{\"x\":\"1995-01-01T00:00:00+00:00\",\"y\":3.5}]},\"description\":\"Area Plot\",\"encoding\":{\"x\":{\"field\":\"x\",\"title\":\"Time\",\"type\":\"temporal\"},\"y\":{\"field\":\"y\",\"title\":\"\",\"type\":\"quantitative\"}},\"mark\":{\"type\":\"area\",\"line\":true,\"point\":true}}",
                "metadata": null
            })
                .to_string()
        );
    }

    fn generate_mock_raster_source(
        time_intervals: Vec<TimeInterval>,
        values_vec: Vec<Vec<u8>>,
    ) -> Box<dyn RasterOperator> {
        assert_eq!(time_intervals.len(), values_vec.len());
        assert!(values_vec.iter().all(|v| v.len() == 6));

        let mut tiles = Vec::with_capacity(time_intervals.len());
        for (time_interval, values) in time_intervals.into_iter().zip(values_vec) {
            tiles.push(RasterTile2D::new_with_tile_info(
                time_interval,
                TileInformation {
                    global_geo_transform: Default::default(),
                    global_tile_position: [0, 0].into(),
                    tile_size_in_pixels: [3, 2].into(),
                },
                Grid2D::new([3, 2].into(), values, None).unwrap(),
            ));
        }

        MockRasterSource {
            params: MockRasterSourceParams {
                data: tiles,
                result_descriptor: RasterResultDescriptor {
                    data_type: RasterDataType::U8,
                    spatial_reference: SpatialReference::epsg_4326().into(),
                    measurement: Measurement::Unitless,
                },
            },
        }
        .boxed()
    }

    #[tokio::test]
    async fn raster_series() {
        let temporal_raster_mean_plot = TemporalRasterMeanPlot {
            params: TemporalRasterMeanPlotParams {
                time_position: TemporalRasterMeanPlotTimePosition::Start,
            },
            raster_sources: vec![generate_mock_raster_source(
                vec![
                    TimeInterval::new(
                        TimeInstance::from(NaiveDate::from_ymd(1990, 1, 1).and_hms(0, 0, 0)),
                        TimeInstance::from(NaiveDate::from_ymd(1995, 1, 1).and_hms(0, 0, 0)),
                    )
                    .unwrap(),
                    TimeInterval::new(
                        TimeInstance::from(NaiveDate::from_ymd(1995, 1, 1).and_hms(0, 0, 0)),
                        TimeInstance::from(NaiveDate::from_ymd(2000, 1, 1).and_hms(0, 0, 0)),
                    )
                    .unwrap(),
                    TimeInterval::new(
                        TimeInstance::from(NaiveDate::from_ymd(2000, 1, 1).and_hms(0, 0, 0)),
                        TimeInstance::from(NaiveDate::from_ymd(2005, 1, 1).and_hms(0, 0, 0)),
                    )
                    .unwrap(),
                ],
                vec![
                    vec![1, 2, 3, 4, 5, 6],
                    vec![9, 9, 8, 8, 8, 9],
                    vec![3, 4, 5, 6, 7, 8],
                ],
            )],
            vector_sources: vec![],
        };

        let execution_context = MockExecutionContext::default();

        let temporal_raster_mean_plot = temporal_raster_mean_plot
            .boxed()
            .initialize(&execution_context)
            .unwrap();

        let processor = temporal_raster_mean_plot
            .query_processor()
            .unwrap()
            .json()
            .unwrap();

        let result = processor
            .plot_query(
                QueryRectangle {
                    bbox: BoundingBox2D::new((-180., -90.).into(), (180., 90.).into()).unwrap(),
                    time_interval: TimeInterval::default(),
                    spatial_resolution: SpatialResolution::one(),
                },
                &MockQueryContext::new(0),
            )
            .await
            .unwrap();

        assert_eq!(
            serde_json::from_value::<PlotData<()>>(result).unwrap(),
            AreaLineChart::new(
                vec![
                    TimeInstance::from(NaiveDate::from_ymd(1990, 1, 1).and_hms(0, 0, 0)),
                    TimeInstance::from(NaiveDate::from_ymd(1995, 1, 1).and_hms(0, 0, 0)),
                    TimeInstance::from(NaiveDate::from_ymd(2000, 1, 1).and_hms(0, 0, 0))
                ],
                vec![3.5, 8.5, 5.5],
                Measurement::Unitless,
            )
            .unwrap()
            .to_vega_embeddable(false)
            .unwrap()
        );
    }
}
