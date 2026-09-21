use thiserror::Error;

use crate::setup;

#[derive(Debug, Error)]
pub(crate) enum CliError {
    #[error("failed to render help")]
    Help {
        #[source]
        source: std::io::Error,
    },
    #[error(transparent)]
    Runtime(#[from] kairo_runtime::RuntimeError),
    #[error("`--input` can only be used with a component")]
    WorkflowInput,
    #[error("`--value` can only be used with a workflow whose input is `io: input: value`")]
    ValueInput,
    #[error("file input can only be used with a stream workflow")]
    StreamInput,
    #[error("workflow `{workflow}` does not take an input")]
    UnexpectedInput { workflow: String },
    #[error("workflow `{workflow}` needs a file input\n\ntry:\n  kairo run {workflow} <file>")]
    MissingStreamInput { workflow: String },
    #[error("`--materialize` can only be used with a stream workflow")]
    Materialize,
    #[error("`--output` requires a stream workflow that declares an output artifact")]
    Output,
    #[error("output already exists: {path}\nuse -o <path> to choose another destination")]
    OutputExists { path: std::path::PathBuf },
    #[error("failed to export output file `{path}`")]
    Export {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("`--cell` and `--state` can only be used with a scalar workflow")]
    State,
    #[error("`--watch` requires a workflow, not a bare component")]
    Watch,
    #[error(
        "`--workers` cannot change an already running service; omit it or restart with `kairo start --workers COUNT`"
    )]
    WatchWorkers,
    #[error("local service has no connected workers; run `kairo start --workers COUNT`")]
    NoWorkers,
    #[error(
        "local service was started by an older Kairo; stop it with Ctrl-C, then run `kairo start`"
    )]
    ServiceUpgrade,
    #[error(transparent)]
    Storage(#[from] kairo_storage::StorageError),
    #[error(transparent)]
    Setup(#[from] setup::SetupError),
    #[error(transparent)]
    Inspection(#[from] crate::inspection::InspectionError),
    #[error(transparent)]
    StatePath(#[from] crate::state::StateError),
    #[error(transparent)]
    New(#[from] crate::new::NewError),
    #[error(transparent)]
    Discovery(#[from] crate::discovery::DiscoveryError),
    #[error(transparent)]
    Tui(#[from] kairo_tui::TuiError),
    #[error(transparent)]
    Control(#[from] kairo_control::ControlError),
    #[error("failed to start local worker")]
    StartWorker {
        #[source]
        source: std::io::Error,
    },
    #[error("effect service failed: {0}")]
    Effect(String),
    #[error("doctor found a problem")]
    Doctor,
    #[error(transparent)]
    StreamRun(#[from] kairo_runtime::StreamRunError),
    #[error(
        "`--workers` is not used by value workflows; use `kairo up --scale COUNT` to configure local workers"
    )]
    ValueWorkers,
    #[error(
        "workflow `{workflow}` needs a value input, and stdin is not a terminal to ask for one\n\ntry:\n  kairo run {workflow} --value <value>"
    )]
    MissingValueInput { workflow: String },
    #[error("failed to read input")]
    Prompt {
        #[source]
        source: std::io::Error,
    },
    #[error("failed to read input `{path}`")]
    ReadInput {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("input `{path}` exceeds the {max_bytes}-byte limit")]
    InputTooLarge {
        path: std::path::PathBuf,
        max_bytes: u64,
    },
    #[error(transparent)]
    Bench(#[from] crate::bench::BenchError),
    #[error(transparent)]
    Component(#[from] crate::component::ComponentError),
}

impl CliError {
    /// a coarse, stable exit-code category for scripts to branch on: 2 for a usage/argument
    /// mistake the caller can fix by changing flags, 4 for a destination conflict, 1 for
    /// everything else (execution, setup, or service failures).
    pub(crate) fn exit_code(&self) -> u8 {
        match self {
            Self::WorkflowInput
            | Self::ValueInput
            | Self::StreamInput
            | Self::UnexpectedInput { .. }
            | Self::MissingStreamInput { .. }
            | Self::Materialize
            | Self::Output
            | Self::State
            | Self::Watch
            | Self::WatchWorkers
            | Self::ValueWorkers
            | Self::MissingValueInput { .. } => 2,
            Self::OutputExists { .. } => 4,
            _ => 1,
        }
    }
}

pub(crate) type Result<T> = std::result::Result<T, CliError>;
