use std::{io, path::PathBuf};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum WorkflowError {
    #[error("failed to open workflow `{path}`")]
    Open {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to read workflow `{path}`")]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("workflow `{path}` exceeds the {max_bytes}-byte size limit")]
    TooLarge { path: PathBuf, max_bytes: usize },
    #[error("workflow YAML is invalid")]
    InvalidYaml {
        #[source]
        source: yaml_serde::Error,
    },
    #[error("workflow is not valid UTF-8")]
    InvalidUtf8 {
        #[source]
        source: std::string::FromUtf8Error,
    },
    #[error("workflow name cannot be empty")]
    EmptyName,
    #[error("workflow description must be at most 200 printable characters")]
    InvalidDescription,
    #[error("workflow aliases must be unique, printable names of at most 64 characters")]
    InvalidAliases,
    #[error(
        "workflow accepted input labels must be unique printable names of at most 32 characters"
    )]
    InvalidAccepts,
    #[error(
        "workflow produced output labels must be unique printable names of at most 32 characters"
    )]
    InvalidProduces,
    #[error("workflow `io.filename` must be a basename of 1-128 printable ASCII characters")]
    InvalidIoFilename,
    #[error("workflow resource limits must be greater than zero")]
    ZeroResources,
    #[error("workflow memory resource limit is too large for this host")]
    ResourceMemoryOverflow,
    #[error("workflow must contain at least one step")]
    NoSteps,
    #[error("workflow contains {steps} steps, exceeding the {max_steps}-step limit")]
    TooManySteps { steps: usize, max_steps: usize },
    #[error("workflow step {index} has an invalid name")]
    InvalidStepName { index: usize },
    #[error("workflow contains duplicate step `{step}`")]
    DuplicateStep { step: String },
    #[error("workflow step `{step}` has an empty component path")]
    EmptyComponentPath { step: String },
    #[error("workflow step `{step}` has an invalid pinned component hash")]
    InvalidPinnedHash { step: String },
    #[error("workflow edge references unknown step `{step}`")]
    UnknownStep { step: String },
    #[error("workflow edge {index} has an empty `{endpoint}` step")]
    EmptyEdgeStep {
        index: usize,
        endpoint: &'static str,
    },
    #[error("workflow contains duplicate edge `{from}` → `{to}`")]
    DuplicateEdge { from: String, to: String },
    #[error("workflow graph contains a cycle")]
    Cycle,
    #[error("workflow graph is disconnected")]
    Disconnected,
    #[error("step `{step}` has multiple inputs; only linear workflows are supported")]
    MultipleInputs { step: String },
    #[error("step `{step}` has multiple outputs; only linear workflows are supported")]
    MultipleOutputs { step: String },
    #[error("scalar workflow input must be an unsigned integer")]
    ScalarInput,
    #[error("stream workflow input must be a file path")]
    StreamInput,
    #[error("value workflow input must be a file path")]
    ValueInput,
    #[error("stream workflows require at least one transform and one consumer")]
    StreamWorkflowSteps,
    #[error("workflow wait must specify exactly one of `timer_ms` or `signal`")]
    InvalidWait,
    #[error("workflow effect operation must be 1–64 letters, digits, `-`, or `_`")]
    InvalidEffect,
    #[error("workflow {kind} references unknown step `{step}`")]
    UnknownBoundaryStep { kind: &'static str, step: String },
    #[error("workflow {kind} after `{step}` requires a durable outgoing edge")]
    InvalidBoundary { kind: &'static str, step: String },
    #[error("workflow wait and effect cannot use the same boundary")]
    ConflictingBoundaries,
    #[error("stream workflows do not support waits or effects")]
    StreamControl,
    #[error("stream result labels must be 1–64 printable characters")]
    InvalidStreamResult,
    #[error("scalar workflows do not support stream result labels")]
    ScalarStreamResult,
    #[error("scalar workflows do not support output artifacts")]
    ScalarOutput,
    #[error("workflow output filename must be a basename of 1-128 printable ASCII characters")]
    InvalidOutputFilename,
    #[error("workflow output content type must be 1-128 printable ASCII characters")]
    InvalidOutputContentType,
}
