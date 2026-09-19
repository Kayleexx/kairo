use std::time::Instant;

use kairo_core::Workflow;
use kairo_storage::ArtifactStore;
use wasmtime::component::StreamReader;

use crate::stream_input::BufferProducer;

use super::{
    Result, Runtime, RuntimeError, StreamEdgeMetrics, StreamInput, StreamMetrics, StreamResult,
    edge_measure::drain_edge_stream, finish_hash,
};

#[derive(Clone, Debug)]
pub enum StreamGroupOutcome {
    Completed(StreamResult),
    Yielded {
        next_index: usize,
        artifact_hash: String,
        artifact_backend: String,
        bytes: u64,
        duration: std::time::Duration,
        metrics: StreamMetrics,
    },
}

/// where a stream ExecutionGroup's own input comes from -- a real local file for the first
/// group, or another group's already-fetched durable checkpoint for every group after it.
pub enum StreamGroupInput<'a> {
    File(&'a std::path::Path),
    Bytes(Vec<u8>),
}

impl Runtime {
    /// runs one stream ExecutionGroup: transforms `start_index..=stop_at` (or through the
    /// terminal when `stop_at` is `None`), mirroring `run_cell_group`'s scalar-mode contract so
    /// `kairo-worker` can drive both mode with the same placement/resume machinery. A non-final
    /// group materializes its boundary into a real durable artifact -- the exact mechanism
    /// `stream::edge_measure` already uses for measurement, reused here for a real durability cut.
    pub async fn run_stream_cell_group(
        &self,
        workflow: &Workflow,
        artifacts: Option<&ArtifactStore>,
        start_index: usize,
        start_input: StreamGroupInput<'_>,
        stop_at: Option<usize>,
    ) -> Result<StreamGroupOutcome> {
        let prepared = self.prepare_stream_workflow(workflow)?;
        let num_transforms = prepared.transforms.len();
        let reaches_terminal = stop_at.is_none_or(|index| index >= num_transforms);
        let transform_end = if reaches_terminal {
            num_transforms
        } else {
            stop_at.map_or(num_transforms, |index| index + 1)
        };

        let mut store = self.new_store(workflow.resources())?;
        store.data_mut().stream_metrics = Some(StreamMetrics {
            edges: workflow
                .edges()
                .iter()
                .map(|edge| StreamEdgeMetrics {
                    name: format!("{} -> {}", edge.from, edge.to),
                    bytes: None,
                    peak_buffered_bytes: None,
                    materialized: None,
                    materialized_bytes: None,
                })
                .collect(),
            ..StreamMetrics::default()
        });

        let (mut input, input_hasher) = match start_input {
            StreamGroupInput::File(path) => {
                let (input, materialized_bytes, hasher) = StreamInput::open(
                    path,
                    self.config.max_stream_input_bytes,
                    self.config.stream_chunk_bytes,
                    false,
                )?;
                if let Some(metrics) = store.data_mut().stream_metrics.as_mut() {
                    metrics.materialized_bytes = materialized_bytes;
                }
                (input.reader(&mut store)?, Some(hasher))
            }
            StreamGroupInput::Bytes(bytes) => {
                let reader = StreamReader::new(
                    &mut store,
                    BufferProducer::from_bytes(bytes, self.config.stream_chunk_bytes),
                )
                .map_err(|source| RuntimeError::CreateStream { source })?;
                (reader, None)
            }
        };

        let started = Instant::now();
        for prepared_transform in &prepared.transforms[start_index..transform_end] {
            let transform = prepared_transform
                .transform
                .instantiate_async(&mut store)
                .await
                .map_err(|source| {
                    self.stream_step_error(
                        &prepared_transform.name,
                        self.instantiation_error(source, &store),
                    )
                })?;
            let call = store
                .run_concurrent(async |accessor| transform.call_transform(accessor, input).await)
                .await;
            input = self.finish_stream_call(call, &store, &prepared_transform.name)?;
        }

        if reaches_terminal {
            let (summary, values, outputs) = self
                .run_stream_terminal(workflow, &prepared, input, &mut store, artifacts)
                .await?;
            let mut metrics = store.data().stream_metrics.clone().unwrap_or_default();
            metrics.consumed_bytes =
                if workflow.stream_result_labels().is_some() || !values.is_empty() {
                    metrics.source_bytes
                } else {
                    summary >> 32
                };
            let input_hash = input_hasher
                .as_ref()
                .map(finish_hash)
                .transpose()?
                .unwrap_or_default();
            return Ok(StreamGroupOutcome::Completed(StreamResult {
                bytes: summary >> 32,
                checksum: summary as u32,
                duration: started.elapsed(),
                metrics,
                input_hash,
                values,
                outputs,
            }));
        }

        let boundary_step = workflow
            .steps()
            .get(transform_end - 1)
            .map_or("durability cut", |step| step.id.as_str());
        let limit = self.config.max_stream_output_bytes;
        let outcome = store
            .run_concurrent(async |accessor| drain_edge_stream(accessor, input, limit).await)
            .await;
        let bytes = match outcome {
            Ok(Ok(bytes)) => bytes,
            Ok(Err(source)) => return Err(self.stream_step_error(boundary_step, source)),
            Err(source) => {
                return Err(self
                    .stream_step_error(boundary_step, RuntimeError::MeasureStreamEdge { source }));
            }
        };
        let artifacts = artifacts.ok_or(RuntimeError::ArtifactStoreRequired)?;
        let artifact = self
            .store_stream_output(&bytes, artifacts, boundary_step)
            .await?;
        Ok(StreamGroupOutcome::Yielded {
            next_index: transform_end,
            artifact_hash: artifact.hash,
            artifact_backend: artifacts.backend().as_str().to_owned(),
            bytes: artifact.bytes,
            duration: started.elapsed(),
            metrics: store.data().stream_metrics.clone().unwrap_or_default(),
        })
    }
}
