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

mod consume_metrics {
    wasmtime::component::bindgen!({
        world: "consume-metrics",
        path: "../../wit/stream.wit",
    });
}

struct PreparedStreamWorkflow {
    transforms: Vec<PreparedTransform>,
    consume_name: String,
    consume_hash: ComponentHash,
    consume: PreparedConsumer,
}

enum PreparedConsumer {
    Plain(consume::ConsumePre<StoreState>),
    Metrics(consume_metrics::ConsumeMetricsPre<StoreState>),
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
    pub values: Vec<StreamValue>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StreamValue {
    pub name: String,
    pub value: u64,
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
        let mut store = self.new_store(workflow.resources())?;
        store.data_mut().stream_metrics = Some(StreamMetrics {
            materialized_bytes,
            ..StreamMetrics::default()
        });

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
            let call = store
                .run_concurrent(async |accessor| transform.call_transform(accessor, input).await)
                .await;
            input = self.finish_stream_call(call, &store, &prepared_transform.name)?;
        }
        let (summary, values) = match prepared.consume {
            PreparedConsumer::Plain(consume) => {
                let consume = consume
                    .instantiate_async(&mut store)
                    .await
                    .map_err(|source| {
                        self.stream_step_error(
                            &prepared.consume_name,
                            self.instantiation_error(source, &store),
                        )
                    })?;
                let call = store
                    .run_concurrent(async |accessor| consume.call_consume(accessor, input).await)
                    .await;
                (
                    self.finish_stream_call(call, &store, &prepared.consume_name)?,
                    Vec::new(),
                )
            }
            PreparedConsumer::Metrics(consume) => {
                let consume = consume
                    .instantiate_async(&mut store)
                    .await
                    .map_err(|source| {
                        self.stream_step_error(
                            &prepared.consume_name,
                            self.instantiation_error(source, &store),
                        )
                    })?;
                let call = store
                    .run_concurrent(async |accessor| consume.call_consume(accessor, input).await)
                    .await;
                let result = self.finish_stream_call(call, &store, &prepared.consume_name)?;
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
                ((high << 32) | u64::from(low), values)
            }
        };
        let mut metrics = store.data().stream_metrics.unwrap_or_default();
        metrics.consumed_bytes = if workflow.stream_result_labels().is_some() || !values.is_empty()
        {
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
            values,
        })
    }

    pub fn validate_stream_workflow(&self, workflow: &Workflow) -> Result<()> {
        self.prepare_stream_workflow(workflow).map(|_| ())
    }

    fn prepare_stream_workflow(&self, workflow: &Workflow) -> Result<PreparedStreamWorkflow> {
        self.validate_workflow_resources(workflow)?;
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
        let plain = linker
            .instantiate_pre(&consume_component.component)
            .ok()
            .and_then(|pre| consume::ConsumePre::new(pre).ok());
        let consume = if let Some(plain) = plain {
            PreparedConsumer::Plain(plain)
        } else {
            let result_pre = linker
                .instantiate_pre(&consume_component.component)
                .map_err(|source| RuntimeError::IncompatibleStreamComponent {
                    step: consume_step.id.to_string(),
                    path: consume_step.component.clone(),
                    role: "consume",
                    source,
                })?;
            PreparedConsumer::Metrics(consume_metrics::ConsumeMetricsPre::new(result_pre).map_err(
                |source| RuntimeError::IncompatibleStreamComponent {
                    step: consume_step.id.to_string(),
                    path: consume_step.component.clone(),
                    role: "consume",
                    source,
                },
            )?)
        };
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

    fn finish_stream_call<T>(
        &self,
        result: wasmtime::Result<wasmtime::Result<T>>,
        store: &wasmtime::Store<StoreState>,
        step: &str,
    ) -> Result<T> {
        match result {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(source)) => Err(self.stream_step_error(
                step,
                self.stream_execution_error(source, store, super::CallKind::Workflow),
            )),
            Err(source) => Err(self.stream_step_error(
                step,
                self.stream_execution_error(source, store, super::CallKind::Runtime),
            )),
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

fn validate_values(
    values: Vec<consume_metrics::kairo::example::stream_metrics::Metric>,
) -> std::result::Result<Vec<StreamValue>, String> {
    if values.is_empty() || values.len() > 16 {
        return Err("component must return between 1 and 16 result values".to_owned());
    }
    values
        .into_iter()
        .map(|value| {
            if value.name.is_empty()
                || value.name.len() > 32
                || !value
                    .name
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
            {
                return Err("component returned an invalid result name".to_owned());
            }
            Ok(StreamValue {
                name: value.name,
                value: value.value,
            })
        })
        .collect()
}
