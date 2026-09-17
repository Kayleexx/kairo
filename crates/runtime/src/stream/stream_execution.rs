use std::{path::Path, time::Instant};

use kairo_core::Workflow;
use kairo_storage::ArtifactStore;
use wasmtime::{Store, component::StreamReader};

use crate::StoreState;

use super::{
    PreparedConsumer, PreparedStreamWorkflow, PreparedTerminal, Runtime, RuntimeError,
    StreamEdgeMetrics, StreamInput, StreamMetrics, StreamResult, StreamValue,
    WorkflowOutputArtifact, finish_hash, validate_values,
};

impl Runtime {
    pub async fn run_stream_workflow(
        &self,
        workflow: &Workflow,
        input: Option<&Path>,
        materialize: bool,
    ) -> super::Result<StreamResult> {
        self.run_stream_workflow_with_artifacts(workflow, input, materialize, None)
            .await
    }

    pub async fn run_stream_workflow_with_artifacts(
        &self,
        workflow: &Workflow,
        input: Option<&Path>,
        materialize: bool,
        artifacts: Option<&ArtifactStore>,
    ) -> super::Result<StreamResult> {
        self.run_stream_workflow_inner(workflow, input, materialize, artifacts, false)
            .await
    }

    /// Temporary MEASUREMENT entry point (Phase 13, Slice 13.3): identical to
    /// `run_stream_workflow_with_artifacts`, except every inter-component edge is drained into
    /// a bounded in-memory buffer so its real byte count can be recorded. Never called by
    /// `kairo run`'s default path -- only by explicit metrics collection (e.g. a benchmark
    /// run). This is not the real streaming architecture: it does not preserve true streaming
    /// backpressure, overlap, or timing. See `stream::edge_measure`'s module docs and the
    /// Phase 13 design notes (prerequisite: a concurrent, bounded-relay streaming graph) for
    /// what the real implementation requires.
    pub async fn measure_stream_workflow_edges(
        &self,
        workflow: &Workflow,
        input: Option<&Path>,
        materialize: bool,
        artifacts: Option<&ArtifactStore>,
    ) -> super::Result<StreamResult> {
        self.run_stream_workflow_inner(workflow, input, materialize, artifacts, true)
            .await
    }

    async fn run_stream_workflow_inner(
        &self,
        workflow: &Workflow,
        input: Option<&Path>,
        materialize: bool,
        artifacts: Option<&ArtifactStore>,
        measure_edges: bool,
    ) -> super::Result<StreamResult> {
        let input = input
            .or_else(|| workflow.stream_input())
            .ok_or(RuntimeError::InvalidStreamWorkflowInput)?;
        let prepared = self.prepare_stream_workflow(workflow)?;
        let (input, materialized_bytes, input_hasher) = StreamInput::open(
            input,
            self.config.max_stream_input_bytes,
            self.config.stream_chunk_bytes,
            materialize,
        )?;
        let mut store = self.new_store(workflow.resources())?;
        store.data_mut().stream_metrics = Some(StreamMetrics {
            materialized_bytes,
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
        let mut input = input.reader(&mut store)?;
        let started = Instant::now();
        for (edge_index, prepared_transform) in prepared.transforms.iter().enumerate() {
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
            let produced = self.finish_stream_call(call, &store, &prepared_transform.name)?;
            input = if measure_edges {
                self.measure_stream_edge(&mut store, &prepared_transform.name, produced, edge_index)
                    .await?
            } else {
                produced
            };
        }
        let (summary, values, outputs) = self
            .run_stream_terminal(workflow, &prepared, input, &mut store, artifacts)
            .await?;
        let mut metrics = store.data().stream_metrics.clone().unwrap_or_default();
        metrics.consumed_bytes = if workflow.stream_result_labels().is_some() || !values.is_empty()
        {
            metrics.source_bytes
        } else {
            summary >> 32
        };
        let duration = started.elapsed();
        let input_hash = finish_hash(&input_hasher)?;
        tracing::info!(
            workflow = workflow.name(), transforms = prepared.transforms.len(), consume_hash = %prepared.consume_hash,
            bytes = summary >> 32, checksum = summary as u32, source_bytes = metrics.source_bytes,
            consumed_bytes = metrics.consumed_bytes, largest_batch_bytes = metrics.largest_batch_bytes,
            materialized_bytes = metrics.materialized_bytes, duration_us = duration.as_micros(),
            "stream workflow executed"
        );
        Ok(StreamResult {
            bytes: summary >> 32,
            checksum: summary as u32,
            duration,
            metrics,
            input_hash,
            values,
            outputs,
        })
    }

    /// runs the consume/output terminal against `input`, shared by both a single-group stream
    /// run and the final group of a multi-group one -- exactly the same terminal-handling logic
    /// either way, never duplicated.
    pub(super) async fn run_stream_terminal(
        &self,
        workflow: &Workflow,
        prepared: &PreparedStreamWorkflow,
        input: StreamReader<u8>,
        store: &mut Store<StoreState>,
        artifacts: Option<&ArtifactStore>,
    ) -> super::Result<(u64, Vec<StreamValue>, Vec<WorkflowOutputArtifact>)> {
        match &prepared.terminal {
            PreparedTerminal::Consumer(PreparedConsumer::Plain(consume)) => {
                let consume = consume
                    .instantiate_async(&mut *store)
                    .await
                    .map_err(|source| {
                        self.stream_step_error(
                            &prepared.consume_name,
                            self.instantiation_error(source, store),
                        )
                    })?;
                let call = store
                    .run_concurrent(async |accessor| consume.call_consume(accessor, input).await)
                    .await;
                Ok((
                    self.finish_stream_call(call, store, &prepared.consume_name)?,
                    Vec::new(),
                    Vec::new(),
                ))
            }
            PreparedTerminal::Consumer(PreparedConsumer::Metrics(consume)) => {
                let consume = consume
                    .instantiate_async(&mut *store)
                    .await
                    .map_err(|source| {
                        self.stream_step_error(
                            &prepared.consume_name,
                            self.instantiation_error(source, store),
                        )
                    })?;
                let call = store
                    .run_concurrent(async |accessor| consume.call_consume(accessor, input).await)
                    .await;
                let result = self.finish_stream_call(call, store, &prepared.consume_name)?;
                let values = result.map_err(|message| {
                    self.stream_step_error(
                        &prepared.consume_name,
                        RuntimeError::StreamInputRejected {
                            message: message.chars().take(1024).collect(),
                        },
                    )
                })?;
                let values = validate_values(values).map_err(|message| {
                    self.stream_step_error(
                        &prepared.consume_name,
                        RuntimeError::StreamInputRejected { message },
                    )
                })?;
                let high = values.first().map_or(0, |value| value.value);
                let low = values.get(1).map_or(0, |value| value.value);
                let low = u32::try_from(low).map_err(|_| {
                    self.stream_step_error(
                        &prepared.consume_name,
                        RuntimeError::StreamInputRejected {
                            message: "second result value exceeds the u32 compatibility limit"
                                .to_owned(),
                        },
                    )
                })?;
                Ok(((high << 32) | u64::from(low), values, Vec::new()))
            }
            PreparedTerminal::Output(transform) => {
                let output = transform
                    .instantiate_async(&mut *store)
                    .await
                    .map_err(|source| {
                        self.stream_step_error(
                            &prepared.consume_name,
                            self.instantiation_error(source, store),
                        )
                    })?;
                let call = store
                    .run_concurrent(async |accessor| output.call_output(accessor, input).await)
                    .await;
                let output = self.finish_stream_call(call, store, &prepared.consume_name)?;
                let output = output.map_err(|message| {
                    self.stream_step_error(
                        &prepared.consume_name,
                        RuntimeError::StreamInputRejected {
                            message: message.chars().take(1024).collect(),
                        },
                    )
                })?;
                let declaration = workflow
                    .output()
                    .ok_or(RuntimeError::InvalidStreamWorkflowInput)?;
                let artifacts = artifacts.ok_or(RuntimeError::ArtifactStoreRequired)?;
                let artifact = self
                    .store_stream_output(&output, artifacts, &prepared.consume_name)
                    .await?;
                let bytes = artifact.bytes;
                Ok((
                    bytes << 32,
                    Vec::new(),
                    vec![WorkflowOutputArtifact {
                        filename: declaration.filename.clone(),
                        content_type: declaration.content_type.clone(),
                        bytes,
                        hash: artifact.hash,
                        backend: artifacts.backend().as_str().to_owned(),
                        reference: artifact.reference,
                        exported_path: None,
                    }],
                ))
            }
        }
    }
}
