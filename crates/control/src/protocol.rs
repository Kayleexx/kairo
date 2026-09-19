use std::fmt;
use std::{collections::BTreeMap, net::SocketAddr, path::PathBuf};

use kairo_storage::StorageConfig;
use serde::{Deserialize, Serialize};

use crate::history::RunEvent;
use crate::{LiveEdgeMetrics, LiveEdgeParticipant, LiveEdgeSession};

mod wire;
pub(crate) use wire::{Request, Response};

/// A completed run's small control-plane result. Stream bytes and artifacts never travel through
/// this protocol; the stream variant carries only the metadata needed by status and inspection.
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(untagged)]
pub enum RunOutput {
    Scalar(u32),
    Stream(StreamOutput),
}

impl fmt::Display for RunOutput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Scalar(value) => value.fmt(formatter),
            Self::Stream(output) if output.values.is_empty() => {
                write!(
                    formatter,
                    "{} bytes · checksum {:08x}",
                    output.bytes, output.checksum
                )
            }
            Self::Stream(output) => {
                let values = output
                    .values
                    .iter()
                    .map(|value| format!("{} {}", value.value, value.name))
                    .collect::<Vec<_>>()
                    .join(" · ");
                formatter.write_str(&values)
            }
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub struct StreamOutput {
    pub bytes: u64,
    pub checksum: u32,
    #[serde(default)]
    pub values: Vec<StreamValue>,
    #[serde(default)]
    pub outputs: Vec<StreamArtifact>,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub struct StreamValue {
    pub name: String,
    pub value: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub struct StreamArtifact {
    pub filename: String,
    pub content_type: String,
    pub bytes: u64,
    pub hash: String,
    pub backend: String,
    pub reference: String,
}

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
    #[serde(default)]
    pub replay_until: Option<usize>,
    /// The immutable completed run whose durable boundary started this child run.
    #[serde(default)]
    pub replay_source: Option<String>,
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
pub struct ReplayLineage {
    /// the direct parent run. the source itself remains immutable.
    pub source_run: String,
    /// the root run that supplied the original input provenance.
    #[serde(default)]
    pub original_run: String,
    pub workflow_hash: String,
    pub components: Vec<ReplayComponent>,
    /// the source input hash when the run began from a file.
    #[serde(default)]
    pub input_hash: Option<String>,
    /// the durability decisions frozen by the source run once it reached a boundary.
    #[serde(default)]
    pub resolved_durability: BTreeMap<usize, bool>,
    #[serde(default)]
    pub boundaries: Vec<GroupResume>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ReplayComponent {
    pub step: String,
    pub path: PathBuf,
    pub hash: String,
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
    /// An explicit local stream input. The workflow declaration remains the fallback, preserving
    /// existing YAML-only runs. A later durable input handoff may replace this with an artifact.
    #[serde(default)]
    pub stream_input: Option<PathBuf>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum WaitRequest {
    Timer { due_ms: u64 },
    Signal { name: String },
}

#[derive(Clone, Debug)]
pub enum WorkerResult {
    Completed(RunOutput),
    LiveCompleted {
        output: RunOutput,
        metrics: LiveEdgeMetrics,
    },
    ReplayCompleted(RunOutput),
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
    #[serde(default)]
    pub live_edge: Option<LiveEdgeAssignment>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LiveEdgeAssignment {
    pub session_id: String,
    pub edge_id: String,
    pub parent_epoch: u64,
    pub producer_endpoint: String,
    pub group: usize,
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
        output: RunOutput,
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
    #[serde(default)]
    pub live_edges: Vec<LiveEdgeSession>,
}
