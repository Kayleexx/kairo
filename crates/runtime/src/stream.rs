use std::time::Duration;

use kairo_core::{ComponentHash, Workflow};
use kairo_storage::ArtifactStore;

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

mod output {
    wasmtime::component::bindgen!({
        world: "output",
        path: "../../wit/stream.wit",
    });
}

mod stream_execution;

struct PreparedStreamWorkflow {
    transforms: Vec<PreparedTransform>,
    consume_name: String,
    consume_hash: ComponentHash,
    terminal: PreparedTerminal,
}

enum PreparedTerminal {
    Consumer(PreparedConsumer),
    Output(output::OutputPre<StoreState>),
}

enum PreparedConsumer {
    Plain(consume::ConsumePre<StoreState>),
    Metrics(consume_metrics::ConsumeMetricsPre<StoreState>),
}

struct PreparedTransform {
    name: String,
    transform: transform::TransformPre<StoreState>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct StreamMetrics {
    pub source_bytes: u64,
    pub consumed_bytes: u64,
    pub largest_batch_bytes: usize,
    pub materialized_bytes: u64,
    pub edges: Vec<StreamEdgeMetrics>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StreamEdgeMetrics {
    pub name: String,
    pub bytes: Option<u64>,
    pub peak_buffered_bytes: Option<u64>,
    pub materialized: Option<bool>,
    pub materialized_bytes: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StreamResult {
    pub bytes: u64,
    pub checksum: u32,
    pub duration: Duration,
    pub metrics: StreamMetrics,
    pub input_hash: String,
    pub values: Vec<StreamValue>,
    pub outputs: Vec<WorkflowOutputArtifact>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StreamValue {
    pub name: String,
    pub value: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkflowOutputArtifact {
    pub filename: String,
    pub content_type: String,
    pub bytes: u64,
    pub hash: String,
    pub backend: String,
    pub reference: String,
    pub exported_path: Option<String>,
}

impl Runtime {
    pub fn validate_stream_workflow(&self, workflow: &Workflow) -> Result<()> {
        self.prepare_stream_workflow(workflow).map(|_| ())
    }

    /// ingests a local file into content-addressed artifact storage, bounded to `max_bytes`.
    /// not yet used by `run_stream_workflow_with_artifacts` -- see `stream_input` module docs.
    pub async fn ingest_stream_input(
        &self,
        path: &std::path::Path,
        artifacts: &ArtifactStore,
        max_bytes: u64,
    ) -> Result<kairo_storage::ByteArtifact> {
        super::stream_input::resolve_stream_input(path, artifacts, max_bytes).await
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
        if workflow.output().is_some() {
            let pre = linker
                .instantiate_pre(&consume_component.component)
                .map_err(|source| RuntimeError::IncompatibleStreamComponent {
                    step: consume_step.id.to_string(),
                    path: consume_step.component.clone(),
                    role: "output transform",
                    source,
                })?;
            let output = output::OutputPre::new(pre).map_err(|source| {
                RuntimeError::IncompatibleStreamComponent {
                    step: consume_step.id.to_string(),
                    path: consume_step.component.clone(),
                    role: "output transform",
                    source,
                }
            })?;
            return Ok(PreparedStreamWorkflow {
                transforms,
                consume_name: consume_step.id.to_string(),
                consume_hash: consume_component.hash,
                terminal: PreparedTerminal::Output(output),
            });
        }
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
            terminal: PreparedTerminal::Consumer(consume),
        })
    }

    async fn store_stream_output(
        &self,
        output: &[u8],
        artifacts: &ArtifactStore,
        step: &str,
    ) -> Result<kairo_storage::ByteArtifact> {
        let writer = artifacts
            .begin_bytes(self.config.max_stream_output_bytes)
            .await
            .map_err(|source| RuntimeError::Artifact { source })?;
        let mut writer = writer;
        if let Err(source) = writer.write(output) {
            writer.abort();
            return Err(self.stream_step_error(step, RuntimeError::Artifact { source }));
        }
        writer
            .finish()
            .await
            .map_err(|source| self.stream_step_error(step, RuntimeError::Artifact { source }))
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
