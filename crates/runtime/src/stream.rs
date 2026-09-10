use std::{
    path::Path,
    time::{Duration, Instant},
};

use kairo_core::{ComponentHash, Workflow};

use super::{
    Result, Runtime, RuntimeError, StoreState,
    stream_input::{StreamInput, StreamLimitFailure, StreamReadFailure, finish_hash},
};

mod transform {
    wasmtime::component::bindgen!({
        world: "transform",
        path: "../../wit/stream.wit",
    });
}

mod consume {
    wasmtime::component::bindgen!({
        world: "consume",
        path: "../../wit/stream.wit",
    });
}

struct PreparedStreamWorkflow {
    transforms: Vec<PreparedTransform>,
    consume_name: String,
    consume_hash: ComponentHash,
    consume: consume::ConsumePre<StoreState>,
}

struct PreparedTransform {
    name: String,
    transform: transform::TransformPre<StoreState>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct StreamMetrics {
    pub source_bytes: u64,
    pub consumed_bytes: u64,
    pub largest_batch_bytes: usize,
    pub materialized_bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StreamResult {
    pub bytes: u64,
    pub checksum: u32,
    pub duration: Duration,
    pub metrics: StreamMetrics,
    pub input_hash: String,
}

impl Runtime {
    pub async fn run_stream_workflow(
        &self,
        workflow: &Workflow,
        input: Option<&Path>,
        materialize: bool,
    ) -> Result<StreamResult> {
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
        let mut store = self.new_store()?;
        store.data_mut().stream_metrics = Some(StreamMetrics {
            materialized_bytes,
            ..StreamMetrics::default()
        });

        let consume = prepared
            .consume
            .instantiate_async(&mut store)
            .await
            .map_err(|source| {
                self.stream_step_error(
                    &prepared.consume_name,
                    self.instantiation_error(source, &store),
                )
            })?;
        let mut input = input.reader(&mut store)?;

        let started = Instant::now();
        for prepared_transform in &prepared.transforms {
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
            input = match store
                .run_concurrent(async |accessor| transform.call_transform(accessor, input).await)
                .await
            {
                Ok(Ok(stream)) => stream,
                Ok(Err(source)) => {
                    return Err(self.stream_step_error(
                        &prepared_transform.name,
                        self.stream_execution_error(source, &store, super::CallKind::Workflow),
                    ));
                }
                Err(source) => {
                    return Err(self.stream_step_error(
                        &prepared_transform.name,
                        self.stream_execution_error(source, &store, super::CallKind::Runtime),
                    ));
                }
            };
        }
        let summary = match store
            .run_concurrent(async |accessor| consume.call_consume(accessor, input).await)
            .await
        {
            Ok(Ok(summary)) => summary,
            Ok(Err(source)) => {
                return Err(self.stream_step_error(
                    &prepared.consume_name,
                    self.stream_execution_error(source, &store, super::CallKind::Workflow),
                ));
            }
            Err(source) => {
                return Err(self.stream_step_error(
                    &prepared.consume_name,
                    self.stream_execution_error(source, &store, super::CallKind::Runtime),
                ));
            }
        };
        let mut metrics = store.data().stream_metrics.unwrap_or_default();
        metrics.consumed_bytes = if workflow.stream_result_labels().is_some() {
            metrics.source_bytes
        } else {
            summary >> 32
        };
        let duration = started.elapsed();
        let input_hash = finish_hash(&input_hasher)?;
        tracing::info!(
            workflow = workflow.name(),
            transforms = prepared.transforms.len(),
            consume_hash = %prepared.consume_hash,
            bytes = summary >> 32,
            checksum = summary as u32,
            source_bytes = metrics.source_bytes,
            consumed_bytes = metrics.consumed_bytes,
            largest_batch_bytes = metrics.largest_batch_bytes,
            materialized_bytes = metrics.materialized_bytes,
            duration_us = duration.as_micros(),
            "stream workflow executed"
        );
        Ok(StreamResult {
            bytes: summary >> 32,
            checksum: summary as u32,
            duration,
            metrics,
            input_hash,
        })
    }

    pub fn validate_stream_workflow(&self, workflow: &Workflow) -> Result<()> {
        self.prepare_stream_workflow(workflow).map(|_| ())
    }

    fn prepare_stream_workflow(&self, workflow: &Workflow) -> Result<PreparedStreamWorkflow> {
        let Some((consume_step, transform_steps)) = workflow.steps().split_last() else {
            return Err(RuntimeError::InvalidStreamWorkflowInput);
        };
        let linker = self.component_linker()?;
        let transforms = transform_steps
            .iter()
            .map(|transform_step| {
                let component = self.load_component(&transform_step.component)?;
                let pre = linker
                    .instantiate_pre(&component.component)
                    .map_err(|source| RuntimeError::IncompatibleStreamComponent {
                        step: transform_step.id.to_string(),
                        path: transform_step.component.clone(),
                        role: "transform",
                        source,
                    })?;
                let transform = transform::TransformPre::new(pre).map_err(|source| {
                    RuntimeError::IncompatibleStreamComponent {
                        step: transform_step.id.to_string(),
                        path: transform_step.component.clone(),
                        role: "transform",
                        source,
                    }
                })?;
                Ok(PreparedTransform {
                    name: transform_step.id.to_string(),
                    transform,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let consume_component = self.load_component(&consume_step.component)?;
        let consume_pre = linker
            .instantiate_pre(&consume_component.component)
            .map_err(|source| RuntimeError::IncompatibleStreamComponent {
                step: consume_step.id.to_string(),
                path: consume_step.component.clone(),
                role: "consume",
                source,
            })?;
        let consume = consume::ConsumePre::new(consume_pre).map_err(|source| {
            RuntimeError::IncompatibleStreamComponent {
                step: consume_step.id.to_string(),
                path: consume_step.component.clone(),
                role: "consume",
                source,
            }
        })?;
        Ok(PreparedStreamWorkflow {
            transforms,
            consume_name: consume_step.id.to_string(),
            consume_hash: consume_component.hash,
            consume,
        })
    }

    fn stream_step_error(&self, step: &str, source: RuntimeError) -> RuntimeError {
        RuntimeError::StreamStep {
            step: step.to_owned(),
            source: Box::new(source),
        }
    }

    fn stream_execution_error(
        &self,
        source: wasmtime::Error,
        store: &wasmtime::Store<StoreState>,
        call: super::CallKind,
    ) -> RuntimeError {
        if let Some(failure) = source.downcast_ref::<StreamReadFailure>() {
            return RuntimeError::StreamInputRead {
                path: failure.path.clone(),
                source,
            };
        }
        if let Some(failure) = source.downcast_ref::<StreamLimitFailure>() {
            return RuntimeError::StreamInputTooLarge {
                path: failure.path.clone(),
                max_bytes: failure.max_bytes,
            };
        }
        self.execution_error(source, store, call)
    }
}
