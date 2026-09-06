use std::{io, path::PathBuf};

use kairo_core::WorkflowError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RuntimeError {
    #[error("failed to initialize Wasmtime")]
    Engine {
        #[source]
        source: wasmtime::Error,
    },
    #[error("failed to open component `{path}`")]
    OpenComponent {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to read component `{path}`")]
    ReadComponent {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("component `{path}` exceeds the {max_bytes}-byte size limit")]
    ComponentTooLarge { path: PathBuf, max_bytes: usize },
    #[error("component `{path}` is invalid")]
    InvalidComponent {
        path: PathBuf,
        #[source]
        source: wasmtime::Error,
    },
    #[error("failed to configure component fuel")]
    ConfigureFuel {
        #[source]
        source: wasmtime::Error,
    },
    #[error("failed to configure the `{capability}` capability")]
    ConfigureCapability {
        capability: &'static str,
        #[source]
        source: wasmtime::Error,
    },
    #[error("failed to instantiate component")]
    Instantiate {
        #[source]
        source: wasmtime::Error,
    },
    #[error("component exhausted its {fuel}-fuel limit")]
    FuelExhausted {
        fuel: u64,
        #[source]
        source: wasmtime::Error,
    },
    #[error("component exceeded its {max_memory_bytes}-byte memory limit")]
    MemoryLimitExceeded {
        max_memory_bytes: usize,
        #[source]
        source: wasmtime::Error,
    },
    #[error("component execution failed")]
    Execute {
        #[source]
        source: wasmtime::Error,
    },
    #[error("failed to invoke `compute`")]
    InvokeComponent {
        #[source]
        source: wasmtime::Error,
    },
    #[error("failed to load workflow `{path}`")]
    LoadWorkflow {
        path: PathBuf,
        #[source]
        source: WorkflowError,
    },
    #[error("component `{path}` for step `{step}` does not implement the workflow stage interface")]
    IncompatibleWorkflowComponent {
        step: String,
        path: PathBuf,
        #[source]
        source: wasmtime::Error,
    },
    #[error("workflow step `{step}` failed")]
    WorkflowStep {
        step: String,
        #[source]
        source: Box<RuntimeError>,
    },
    #[error("failed to invoke the workflow stage")]
    InvokeWorkflow {
        #[source]
        source: wasmtime::Error,
    },
    #[error("stream chunk size must be greater than zero")]
    InvalidStreamChunkSize,
    #[error("failed to open stream input `{path}`")]
    OpenStreamInput {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to read stream input `{path}`")]
    ReadStreamInput {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to read stream input `{path}`")]
    StreamInputRead {
        path: PathBuf,
        #[source]
        source: wasmtime::Error,
    },
    #[error("stream input `{path}` exceeds the {max_bytes}-byte size limit")]
    StreamInputTooLarge { path: PathBuf, max_bytes: u64 },
    #[error("stream workflow has an invalid input")]
    InvalidStreamWorkflowInput,
    #[error("scalar workflow has an invalid input")]
    InvalidScalarWorkflowInput,
    #[error("component `{path}` for step `{step}` does not implement the stream {role} interface")]
    IncompatibleStreamComponent {
        step: String,
        path: PathBuf,
        role: &'static str,
        #[source]
        source: wasmtime::Error,
    },
    #[error("stream step `{step}` failed")]
    StreamStep {
        step: String,
        #[source]
        source: Box<RuntimeError>,
    },
    #[error("failed to create the input stream")]
    CreateStream {
        #[source]
        source: wasmtime::Error,
    },
}

pub type Result<T> = std::result::Result<T, RuntimeError>;
