use std::{collections::BTreeMap, net::SocketAddr, path::PathBuf};

use kairo_storage::StorageConfig;
use serde::{Deserialize, Serialize};

use crate::history::RunEvent;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Endpoint {
    pub address: SocketAddr,
    pub token: String,
}

/// a run's durability plan, resolved once (covering both declared `required` edges and any
/// `durability: auto` edge's resolution) and reused by every ExecutionGroup -- never
/// recomputed mid-run.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct RunPlan {
    pub resolved_durability: BTreeMap<usize, bool>,
}

/// where the next ExecutionGroup should resume from: the boundary step index and the
/// already-committed artifact that carries its input.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct GroupResume {
    pub from_index: usize,
    pub artifact_hash: String,
    pub artifact_backend: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RunRequest {
    pub id: String,
    pub workflow: PathBuf,
    pub state: PathBuf,
    pub storage: Option<StorageConfig>,
    #[serde(default)]
    pub wait: Option<WaitRequest>,
    #[serde(default)]
    pub plan: Option<RunPlan>,
    #[serde(default)]
    pub resume: Option<GroupResume>,
    /// set only on a queued ExecutionGroup continuation -- the worker gate 3 selected as the
    /// best placement. Honored by `Next` until `preferred_deadline_ms` passes, after which any
    /// capable worker may take it, so a placement choice never causes starvation.
    #[serde(default)]
    pub preferred_worker: Option<String>,
    #[serde(default)]
    pub preferred_deadline_ms: Option<u64>,
    /// the workflow's identity for co-location scoring (see `RunEvent::Outcome` correlation in
    /// `Snapshot`) -- known only once a worker has computed it, set on first yield.
    #[serde(default)]
    pub shape: Option<String>,
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
    /// an ExecutionGroup boundary was reached; the run's remainder goes back through the queue.
    Yielded {
        next_index: usize,
        artifact_hash: String,
        artifact_backend: String,
        plan: Option<RunPlan>,
        shape: Option<String>,
        target_worker: Option<String>,
        target_had_cache: bool,
    },
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
    #[serde(default)]
    pub shape: Option<String>,
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
    Yield {
        worker: String,
        token: String,
        id: String,
        epoch: u64,
        next_index: usize,
        artifact_hash: String,
        artifact_backend: String,
        #[serde(default)]
        plan: Option<RunPlan>,
        #[serde(default)]
        shape: Option<String>,
        #[serde(default)]
        target_worker: Option<String>,
        #[serde(default)]
        target_had_cache: bool,
    },
    Submit {
        token: String,
        run: RunRequest,
    },
    Cancel {
        token: String,
        id: String,
    },
    /// removes a finished run's record entirely, for callers (like `kairo bench --profile`) that
    /// submit many short-lived internal runs and don't want them left behind as real user-visible
    /// history. Refused for anything not yet in a terminal state.
    Forget {
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
            | Self::Yield { token, .. }
            | Self::Submit { token, .. }
            | Self::Cancel { token, .. }
            | Self::Forget { token, .. }
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
    Assignment { run: Option<Box<Assignment>> },
    Status { status: Option<RunStatus> },
    Snapshot { snapshot: Snapshot },
    Error { message: String },
}
