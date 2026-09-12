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
    #[error("file input can only be used with a stream workflow")]
    StreamInput,
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
    #[error("`--watch` can only be used with a scalar workflow")]
    Watch,
    #[error(
        "`--workers` cannot change an already running service; omit it or restart with `kairo start --workers COUNT`"
    )]
    WatchWorkers,
    #[error("local service has no connected workers; run `kairo start --workers COUNT`")]
    NoWorkers,
    #[error("local service thread stopped unexpectedly")]
    ServiceThread,
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
    #[error("`--workers` is not used by local stream workflows")]
    StreamWorkers,
}

pub(crate) type Result<T> = std::result::Result<T, CliError>;
