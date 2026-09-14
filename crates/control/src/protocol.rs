use std::{net::SocketAddr, path::PathBuf};

use kairo_storage::StorageConfig;
use serde::{Deserialize, Serialize};

use crate::history::RunEvent;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Endpoint {
    pub address: SocketAddr,
    pub token: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RunRequest {
    pub id: String,
    pub workflow: PathBuf,
    pub state: PathBuf,
    pub storage: Option<StorageConfig>,
    #[serde(default)]
    pub wait: Option<WaitRequest>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum WaitRequest {
    Timer { due_ms: u64 },
    Signal { name: String },
}

#[derive(Clone, Debug)]
pub enum WorkerResult {
    Completed(u32),
    Waiting(WaitRequest),
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Assignment {
    pub run: RunRequest,
    pub epoch: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum RunStatus {
    Queued,
    Running {
        worker: String,
        epoch: u64,
    },
    CancelRequested {
        worker: String,
        epoch: u64,
    },
    Canceled,
    Completed {
        output: u32,
        #[serde(default)]
        worker: String,
    },
    Failed {
        message: String,
    },
    Waiting {
        reason: String,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct WorkerSnapshot {
    pub id: String,
    pub busy: bool,
    pub healthy: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RunSnapshot {
    pub id: String,
    pub status: RunStatus,
    #[serde(default)]
    pub history: Vec<RunEvent>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Snapshot {
    pub workers: Vec<WorkerSnapshot>,
    pub runs: Vec<RunSnapshot>,
}

#[derive(Deserialize, Serialize)]
pub(crate) enum Request {
    Register {
        worker: String,
        pid: u32,
        token: String,
    },
    Heartbeat {
        worker: String,
        token: String,
    },
    Next {
        worker: String,
        token: String,
    },
    Complete {
        worker: String,
        token: String,
        id: String,
        epoch: u64,
        output: u32,
    },
    Fail {
        worker: String,
        token: String,
        id: String,
        epoch: u64,
        message: String,
    },
    Wait {
        worker: String,
        token: String,
        id: String,
        epoch: u64,
        wait: WaitRequest,
    },
    Submit {
        token: String,
        run: RunRequest,
    },
    Cancel {
        token: String,
        id: String,
    },
    Status {
        token: String,
        id: String,
    },
    Snapshot {
        token: String,
    },
    Signal {
        token: String,
        id: String,
        signal: String,
    },
    ChaosKill {
        token: String,
        worker: String,
    },
    Shutdown {
        token: String,
    },
}

impl Request {
    pub(crate) fn token(&self) -> &str {
        match self {
            Self::Register { token, .. }
            | Self::Heartbeat { token, .. }
            | Self::Next { token, .. }
            | Self::Complete { token, .. }
            | Self::Fail { token, .. }
            | Self::Wait { token, .. }
            | Self::Submit { token, .. }
            | Self::Cancel { token, .. }
            | Self::Status { token, .. }
            | Self::Snapshot { token }
            | Self::Signal { token, .. }
            | Self::ChaosKill { token, .. }
            | Self::Shutdown { token } => token,
        }
    }
}

#[derive(Deserialize, Serialize)]
pub(crate) enum Response {
    Ok,
    Canceled,
    Assignment { run: Option<Assignment> },
    Status { status: Option<RunStatus> },
    Snapshot { snapshot: Snapshot },
    Error { message: String },
}
