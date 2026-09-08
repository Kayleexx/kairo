use std::path::PathBuf;

use thiserror::Error;

mod client;
mod protocol;
mod server;

pub use client::{snapshot, status, submit, worker_loop};
pub use protocol::{Endpoint, RunRequest, RunSnapshot, RunStatus, Snapshot, WorkerSnapshot};
pub(crate) use protocol::{Request, Response};
pub use server::{Server, load_endpoint};

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
    #[error("system random source failed")]
    Random {
        #[source]
        source: getrandom::Error,
    },
}
