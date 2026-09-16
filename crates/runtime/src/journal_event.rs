use rusqlite::Row;

use crate::journal::JournalError;
use crate::payload::EventPayload;

mod decode;
mod decode_fields;

pub(crate) enum JournalEvent {
    WorkflowStarted {
        name: Option<String>,
        fingerprint: String,
        input: EventPayload,
        component_count: Option<usize>,
        // the ExecutionGroup this journal begins at; absent means index 0 / the workflow input,
        // matching every journal written before ExecutionGroups existed.
        start_index: Option<usize>,
        start_input: Option<EventPayload>,
    },
    ComponentStarted {
        index: usize,
        name: String,
        hash: String,
        input: EventPayload,
        durable_after: Option<bool>,
    },
    ComponentCompleted {
        index: usize,
        output: EventPayload,
        duration_us: Option<u64>,
    },
    CheckpointCreated {
        index: usize,
        hash: String,
        backend: Option<String>,
        bytes: Option<u64>,
        duration_us: Option<u64>,
    },
    WorkflowCompleted {
        output: EventPayload,
    },
    RecoveryTimed {
        duration_us: u64,
    },
    DurabilityPlanned {
        index: usize,
        required: bool,
        profile_id: String,
        reason: String,
        // the profile numbers the planner actually compared -- absent only for journals written
        // before schema v9, never a synthesized/guessed value.
        recompute_us: Option<u64>,
        checkpoint_us: Option<u64>,
        checkpoint_bytes: Option<u64>,
        samples: Option<u32>,
    },
}

pub(crate) fn decode_row(row: &Row<'_>) -> Result<(i64, JournalEvent), JournalError> {
    let (sequence, stored) = decode::from_row(row)?;
    decode::event(stored).map(|event| (sequence, event))
}
