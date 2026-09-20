use std::{io, path::PathBuf};

use kairo_core::{ComponentHash, WorkflowError};
use kairo_storage::StorageError;
use thiserror::Error;

use crate::{JournalError, payload::LocalBlobError};

#[derive(Debug, Error)]
pub enum RuntimeError {
    #[error("failed to configure the Wasmtime component cache")]
    Cache {
        #[source]
        source: wasmtime::Error,
    },
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
    #[error("failed to decode the WIT contract for component `{path}`: {message}")]
    DecodeComponentContract { path: PathBuf, message: String },
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
    #[error("component execution failed: {source}")]
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
    #[error("workflow step `{step}` failed: {source}")]
    WorkflowStep {
        step: String,
        #[source]
        source: Box<RuntimeError>,
    },
    #[error(
        "component at `{path}` for step `{step}` has changed since this workflow was composed \
         (expected {expected}, found {found})"
    )]
    PinnedComponentMismatch {
        step: String,
        path: PathBuf,
        expected: ComponentHash,
        found: ComponentHash,
    },
    #[error("failed to use local workflow state `{path}`")]
    Journal {
        path: PathBuf,
        #[source]
        source: JournalError,
    },
    #[error("local workflow state is not supported for stream workflows")]
    StatefulStreamWorkflow,
    #[error("a workflow with `durability: required` needs an artifact store")]
    ArtifactStoreRequired,
    #[error(
        "step `{step}` uses `durability: auto` but no compatible profile exists; run `kairo workflow profile <workflow>` to measure it"
    )]
    DurabilityProfileMissing { step: String },
    #[error("failed to write the durability profile")]
    ProfileWrite {
        #[source]
        source: io::Error,
    },
    #[error("failed to use a durable artifact")]
    Artifact {
        #[source]
        source: StorageError,
    },
    #[error("checkpoint `{hash}` does not match the journaled value")]
    CheckpointMismatch {
        hash: String,
        expected: u32,
        found: u32,
    },
    #[error(
        "checkpoint `{hash}` was stored in `{recorded}` storage, but `{configured}` is configured"
    )]
    CheckpointBackendMismatch {
        hash: String,
        recorded: String,
        configured: &'static str,
    },
    #[error("failed to invoke the workflow stage")]
    InvokeWorkflow {
        #[source]
        source: wasmtime::Error,
    },
    #[error("stream chunk size must be greater than zero")]
    InvalidStreamChunkSize,
    #[error("workflow resource maxima must cover Kairo's default execution limits")]
    InvalidWorkflowResourceMaximum,
    #[error(
        "workflow requests {requested} fuel, exceeding the host maximum of {maximum}; lower `resources.fuel` or raise the host limit"
    )]
    WorkflowFuelLimit { requested: u64, maximum: u64 },
    #[error(
        "workflow requests {requested} bytes of memory, exceeding the host maximum of {maximum}; lower `resources.memory_bytes` or raise the host limit"
    )]
    WorkflowMemoryLimit { requested: usize, maximum: usize },
    #[error("failed to open stream input `{path}`")]
    OpenStreamInput {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("stream input `{path}` was not found")]
    StreamInputNotFound { path: PathBuf },
    #[error("stream input `{path}` is not a regular file")]
    StreamInputNotRegular { path: PathBuf },
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
    #[error("failed to calculate stream input identity")]
    InputHash,
    #[error("stream workflow has an invalid input")]
    InvalidStreamWorkflowInput,
    #[error("scalar workflow has an invalid input")]
    InvalidScalarWorkflowInput,
    #[error("workflow paused without a requested boundary")]
    UnexpectedPause,
    #[error("component `{path}` for step `{step}` does not implement the stream {role} interface")]
    IncompatibleStreamComponent {
        step: String,
        path: PathBuf,
        role: &'static str,
        #[source]
        source: wasmtime::Error,
    },
    #[error("stream step `{step}` failed: {source}")]
    StreamStep {
        step: String,
        #[source]
        source: Box<RuntimeError>,
    },
    #[error("component rejected the stream input: {message}")]
    StreamInputRejected { message: String },
    #[error("failed to create the input stream: {source}")]
    CreateStream {
        #[source]
        source: wasmtime::Error,
    },
    #[error("failed to pipe the component stream into the live relay: {source}")]
    RelayPipe {
        #[source]
        source: wasmtime::Error,
    },
    #[error("live relay ended: {message}")]
    RelayClosed { message: String },
    #[error("failed to run the component stream relay: {source}")]
    RelayConcurrent {
        #[source]
        source: wasmtime::Error,
    },
    #[error("failed to measure a stream edge")]
    MeasureStreamEdge {
        #[source]
        source: wasmtime::Error,
    },
    #[error("stream edge exceeds the {max_bytes}-byte measurement limit")]
    StreamEdgeTooLarge { max_bytes: u64 },
    #[error("value workflow step `{step}` rejected its input: {message}")]
    ValueStepRejected { step: String, message: String },
    #[error("failed to use a local blob")]
    LocalBlob {
        #[source]
        source: LocalBlobError,
    },
}

pub type Result<T> = std::result::Result<T, RuntimeError>;
