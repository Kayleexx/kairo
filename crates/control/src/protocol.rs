use std::{net::SocketAddr, path::PathBuf};

use kairo_storage::StorageConfig;
use serde::{Deserialize, Serialize};

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
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum RunStatus {
    Queued,
    Running {
        worker: String,
    },
    Completed {
        output: u32,
        #[serde(default)]
        worker: String,
    },
    Failed {
        message: String,
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
        output: u32,
    },
    Fail {
        worker: String,
        token: String,
        id: String,
        message: String,
    },
    Submit {
        token: String,
        run: RunRequest,
    },
    Status {
        token: String,
        id: String,
    },
    Snapshot {
        token: String,
    },
}

#[derive(Deserialize, Serialize)]
pub(crate) enum Response {
    Ok,
    Assignment { run: Option<RunRequest> },
    Status { status: Option<RunStatus> },
    Snapshot { snapshot: Snapshot },
    Error { message: String },
}
