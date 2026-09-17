use std::path::Path;

use rusqlite::{Connection, OpenFlags};

use crate::{
    inspection::{READ_TIMEOUT, classify_read_error, corrupt, event_query},
    journal::JournalError,
    journal_event::{JournalEvent, decode_row},
    payload::EventPayload,
};

mod replay;
mod replay_execution;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValueComponentInspection {
    pub index: usize,
    pub name: String,
    pub hash: String,
    pub input_preview: String,
    pub output_preview: Option<String>,
    pub duration_us: Option<u64>,
    pub durable_after: Option<bool>,
    pub checkpoint: Option<String>,
    pub checkpoint_backend: Option<String>,
    pub checkpoint_bytes: Option<u64>,
    pub checkpoint_duration_us: Option<u64>,
    pub durability_reason: Option<String>,
    pub planner_profile_id: Option<String>,
    pub planner_recompute_us: Option<u64>,
    pub planner_checkpoint_us: Option<u64>,
    pub planner_checkpoint_bytes: Option<u64>,
    pub planner_samples: Option<u32>,
    pub planner_recorded_at_ms: Option<u64>,
    pub attempts: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ValueRunStatus {
    Ready { next_index: usize },
    Interrupted { step: String },
    CheckpointPending { step: String },
    Finalizing,
    Completed { output_preview: String },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValueRunInspection {
    pub name: Option<String>,
    pub input_preview: String,
    pub status: ValueRunStatus,
    pub components: Vec<ValueComponentInspection>,
    pub metadata_complete: bool,
    pub recovery_duration_us: Option<u64>,
}

/// `None` when the journal's very first recorded input is a `Scalar` payload -- i.e. a
/// legacy/scalar-mode journal, which `inspect_cell` already handles. A value-mode workflow's
/// writer (`run_value_cell`) never produces a `Scalar` payload, so this distinction is exact,
/// not a heuristic.
pub fn inspect_value_cell(
    path: impl AsRef<Path>,
) -> Result<Option<ValueRunInspection>, JournalError> {
    let path = path.as_ref();
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|source| JournalError::Open { source })?;
    connection
        .busy_timeout(READ_TIMEOUT)
        .map_err(|source| JournalError::Configure { source })?;
    let version = connection
        .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
        .map_err(classify_read_error)?;
    let query = event_query(version)?;
    let mut statement = connection.prepare(&query).map_err(classify_read_error)?;
    let mut rows = statement.query([]).map_err(classify_read_error)?;
    let mut builder = None;
    let mut expected_sequence = 1_i64;
    while let Some(row) = rows.next().map_err(classify_read_error)? {
        let (sequence, event) = decode_row(row)?;
        if let JournalEvent::WorkflowStarted { ref input, .. } = event
            && matches!(input, EventPayload::Scalar(_))
        {
            return Ok(None);
        }
        if sequence != expected_sequence {
            return Err(corrupt(sequence, "event sequence is not contiguous"));
        }
        expected_sequence = expected_sequence
            .checked_add(1)
            .ok_or_else(|| corrupt(sequence, "event sequence overflow"))?;
        replay::apply_event(sequence, event, &mut builder)?;
    }
    let builder = builder.ok_or_else(|| corrupt(0, "journal has no workflow start"))?;
    Ok(Some(builder.finish()))
}

fn preview(payload: &EventPayload) -> String {
    match payload {
        EventPayload::Scalar(value) => value.to_string(),
        EventPayload::Inline(bytes) => preview_bytes(bytes),
        EventPayload::Local { bytes, .. } => format!("<{bytes} bytes>"),
        EventPayload::Reuse => "(reused)".to_owned(),
    }
}

fn preview_bytes(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(text) if bytes.len() <= 64 => text.to_owned(),
        Ok(text) => format!(
            "{}… ({} bytes)",
            text.chars().take(64).collect::<String>(),
            bytes.len()
        ),
        Err(_) => format!("<{} bytes>", bytes.len()),
    }
}
