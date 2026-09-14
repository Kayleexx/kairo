use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// why a run most recently entered the queue, carried onto its next assignment event.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub enum AssignmentReason {
    Initial,
    ReassignedAfterLeaseExpiry,
    ResumedAfterWait,
    ResumedAfterRestart,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub enum RunOutcome {
    Completed,
    Failed,
    Canceled,
}

/// a real, timestamped transition in a run's life -- never invented or backfilled.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum RunEvent {
    Queued {
        at_ms: u64,
        reason: AssignmentReason,
    },
    Assigned {
        worker: String,
        epoch: u64,
        at_ms: u64,
        reason: AssignmentReason,
    },
    Outcome {
        worker: String,
        epoch: u64,
        at_ms: u64,
        outcome: RunOutcome,
    },
}

/// caps how many events a single run keeps -- this is diagnostic history, not a durability log.
const MAX_EVENTS_PER_RUN: usize = 32;

pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            elapsed.as_millis().min(u128::from(u64::MAX)) as u64
        })
}

pub(crate) fn push(history: &mut Vec<RunEvent>, event: RunEvent) {
    history.push(event);
    if history.len() > MAX_EVENTS_PER_RUN {
        history.remove(0);
    }
}
