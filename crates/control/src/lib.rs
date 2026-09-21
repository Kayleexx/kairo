use std::path::PathBuf;

use thiserror::Error;

mod chaos;
mod client;
mod finish;
mod history;
mod leases;
mod lifecycle;
mod live_edge;
mod persistence;
mod protocol;
mod protocol_io;
mod server;
mod state;
mod submission;

pub use client::{
    begin_live_edge, cancel, complete_live_edge, complete_live_edge_output,
    complete_live_edge_with_metrics, fail_live_edge, forget, kill_worker, live_edge,
    ready_live_edge, shutdown, signal, snapshot, status, streaming_live_edge, submit,
    submit_with_lineage, worker_loop, worker_loop_with_assignments, worker_loop_with_waits,
    yield_group,
};
pub use history::{AssignmentReason, RunEvent, RunOutcome};
pub use lifecycle::{
    LocalEffect, LocalService, ensure_effect_service, ensure_endpoint, start_worker, stop_workers,
};
pub use live_edge::{
    LiveEdgeMetrics, LiveEdgeObservation, LiveEdgeParticipant, LiveEdgeSession, LiveEdgeState,
    LiveEdgeTransitionError,
};
pub use protocol::{
    Assignment, Endpoint, GroupResume, LiveEdgeAssignment, ReplayComponent, ReplayLineage,
    RunOutput, RunPlan, RunRequest, RunSnapshot, RunStatus, Snapshot, StreamArtifact, StreamOutput,
    StreamValue, WaitRequest, WorkerResult, WorkerSnapshot,
};
pub(crate) use protocol::{Request, Response};
pub use server::{Server, load_endpoint};
pub use submission::{
    ReplaySource, SubmissionOutcome, await_run, prepare_replay, replay_source, submit_run,
    submit_stream_run,
};

#[derive(Debug, Error)]
pub enum ControlError {
    #[error("failed to create control directory `{path}`")]
    CreateDirectory {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to bind local control service")]
    Bind {
        #[source]
        source: std::io::Error,
    },
    #[error("failed to read or write control service state")]
    Io {
        #[source]
        source: std::io::Error,
    },
    #[error("failed to resolve a local run path")]
    ResolvePath {
        #[source]
        source: std::io::Error,
    },
    #[error("control protocol message is invalid")]
    Protocol {
        #[source]
        source: serde_json::Error,
    },
    #[error("control protocol message is too large")]
    TooLarge,
    #[error("control service is unavailable")]
    Unavailable,
    #[error("control service rejected the request: {message}")]
    Rejected { message: String },
    #[error("control state is unavailable")]
    State,
    #[error("run `{id}` was not found; run `kairo runs` to see completed runs eligible for replay")]
    RunNotFound { id: String },
    #[error("system random source failed")]
    Random {
        #[source]
        source: getrandom::Error,
    },
    #[error("a worker count was requested, but a local service is already running")]
    WorkersIgnored,
    #[error("a local service is running but has no healthy workers")]
    NoHealthyWorkers,
    #[error("failed to start a local worker")]
    StartWorker {
        #[source]
        source: std::io::Error,
    },
    #[error("local service thread stopped unexpectedly")]
    ServiceThread,
    #[error("local effect service failed: {0}")]
    EffectService(String),
}
