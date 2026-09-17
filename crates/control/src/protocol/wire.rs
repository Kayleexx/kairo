use serde::{Deserialize, Serialize};

use super::{
    Assignment, LiveEdgeMetrics, LiveEdgeParticipant, LiveEdgeSession, RunOutput, RunPlan,
    RunRequest, RunStatus, Snapshot, WaitRequest,
};

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
    LiveEdgeBegin {
        worker: String,
        token: String,
        session_id: String,
        id: String,
        edge_id: String,
        epoch: u64,
        producer_group: usize,
        consumer_group: usize,
        consumer_worker: String,
    },
    LiveEdgeReady {
        worker: String,
        token: String,
        session_id: String,
        id: String,
        edge_id: String,
        epoch: u64,
        producer_group: usize,
        endpoint: String,
    },
    LiveEdgeStreaming {
        worker: String,
        token: String,
        session_id: String,
        id: String,
        edge_id: String,
        epoch: u64,
        consumer_group: usize,
    },
    LiveEdgeComplete {
        worker: String,
        token: String,
        session_id: String,
        id: String,
        edge_id: String,
        epoch: u64,
        group: usize,
        participant: LiveEdgeParticipant,
        #[serde(default)]
        output: Option<RunOutput>,
        #[serde(default)]
        metrics: Option<LiveEdgeMetrics>,
    },
    LiveEdgeFail {
        worker: String,
        token: String,
        session_id: String,
        id: String,
        edge_id: String,
        epoch: u64,
        group: usize,
        participant: LiveEdgeParticipant,
        message: String,
    },
    LiveEdgeStatus {
        token: String,
        session_id: String,
    },
    Complete {
        worker: String,
        token: String,
        id: String,
        epoch: u64,
        output: RunOutput,
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
            | Self::LiveEdgeBegin { token, .. }
            | Self::LiveEdgeReady { token, .. }
            | Self::LiveEdgeStreaming { token, .. }
            | Self::LiveEdgeComplete { token, .. }
            | Self::LiveEdgeFail { token, .. }
            | Self::LiveEdgeStatus { token, .. }
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
    Assignment {
        run: Option<Box<Assignment>>,
    },
    Status {
        status: Option<RunStatus>,
    },
    Snapshot {
        snapshot: Snapshot,
    },
    LiveEdge {
        session: Box<Option<LiveEdgeSession>>,
    },
    Error {
        message: String,
    },
}
