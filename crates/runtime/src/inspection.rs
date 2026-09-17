use std::{path::Path, time::Duration};

use rusqlite::{Connection, ErrorCode, OpenFlags};

use crate::{
    journal::{JournalError, SCHEMA_VERSION},
    journal_event::decode_row,
};

mod replay;

pub(crate) const READ_TIMEOUT: Duration = Duration::from_millis(250);

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CellStatus {
    Ready { next_index: usize },
    Interrupted { step: String },
    CheckpointPending { step: String },
    Finalizing,
    Completed { output: u32 },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ComponentInspection {
    pub index: usize,
    pub name: String,
    pub hash: String,
    pub input: u32,
    pub output: Option<u32>,
    pub duration_us: Option<u64>,
    pub durable_after: Option<bool>,
    pub checkpoint: Option<String>,
    pub checkpoint_backend: Option<String>,
    pub checkpoint_bytes: Option<u64>,
    pub checkpoint_duration_us: Option<u64>,
    /// present only when this step's edge was `durability: auto` and a planner resolved it --
    /// `"<profile id> · <reason>"`.
    pub durability_reason: Option<String>,
    pub planner_profile_id: Option<String>,
    /// the profile numbers the planner compared to reach `durability_reason` -- an estimate
    /// (measured by an earlier profiling run, used predictively for this one), never a fact this
    /// run itself recorded. `None` for a declared (non-auto) edge, or a journal written before
    /// schema v9.
    pub planner_recompute_us: Option<u64>,
    pub planner_checkpoint_us: Option<u64>,
    pub planner_checkpoint_bytes: Option<u64>,
    pub planner_samples: Option<u32>,
    /// when the profile behind `durability_reason` was last updated, if journaled (absent for a
    /// journal written before this field existed).
    pub planner_recorded_at_ms: Option<u64>,
    pub attempts: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CellInspection {
    pub name: Option<String>,
    pub input: u32,
    pub status: CellStatus,
    pub components: Vec<ComponentInspection>,
    pub metadata_complete: bool,
    /// how long the most recent recovery (journal replay on resume) took, if this Cell has ever
    /// been resumed from an existing journal. `None` for a Cell that has never needed recovery.
    pub recovery_duration_us: Option<u64>,
}

pub fn inspect_cell(path: impl AsRef<Path>) -> Result<CellInspection, JournalError> {
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
        if sequence != expected_sequence {
            return Err(corrupt(sequence, "event sequence is not contiguous"));
        }
        expected_sequence = expected_sequence
            .checked_add(1)
            .ok_or_else(|| corrupt(sequence, "event sequence overflow"))?;
        replay::apply_event(sequence, event, &mut builder)?;
    }

    builder
        .ok_or_else(|| corrupt(0, "journal has no workflow start"))?
        .finish()
}

// journals older than the current schema have none of the 12 `payload_*` columns; every version
// arm below except the current one pads them out with `NULL`, same as it already does for any
// other column that didn't exist yet at that version.
const NO_PAYLOAD_COLUMNS: &str =
    "NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL";

pub(crate) fn event_query(version: i64) -> Result<String, JournalError> {
    let query = match version {
        1 => format!(
            "SELECT sequence, kind, step_index, workflow_fingerprint, component_name, \
                 component_hash, input_value, output_value, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, \
                 {NO_PAYLOAD_COLUMNS}, NULL \
                 FROM events ORDER BY sequence",
        ),
        2 => format!(
            "SELECT sequence, kind, step_index, workflow_fingerprint, component_name, \
                 component_hash, input_value, output_value, artifact_hash, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, \
                 {NO_PAYLOAD_COLUMNS}, NULL \
                 FROM events ORDER BY sequence",
        ),
        3 => format!(
            "SELECT sequence, kind, step_index, workflow_fingerprint, component_name, \
             component_hash, input_value, output_value, artifact_hash, workflow_name, \
             duration_us, durability_required, component_count, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, \
             {NO_PAYLOAD_COLUMNS}, NULL FROM events ORDER BY sequence",
        ),
        4 => format!(
            "SELECT sequence, kind, step_index, workflow_fingerprint, component_name, \
             component_hash, input_value, output_value, artifact_hash, workflow_name, \
             duration_us, durability_required, component_count, artifact_backend, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, \
             {NO_PAYLOAD_COLUMNS}, NULL \
             FROM events ORDER BY sequence",
        ),
        5 => format!(
            "SELECT sequence, kind, step_index, workflow_fingerprint, component_name, \
             component_hash, input_value, output_value, artifact_hash, workflow_name, \
             duration_us, durability_required, component_count, artifact_backend, artifact_bytes, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, \
             {NO_PAYLOAD_COLUMNS}, NULL \
             FROM events ORDER BY sequence",
        ),
        6 => format!(
            "SELECT sequence, kind, step_index, workflow_fingerprint, component_name, \
             component_hash, input_value, output_value, artifact_hash, workflow_name, \
             duration_us, durability_required, component_count, artifact_backend, artifact_bytes, \
             planner_profile_id, planner_reason, NULL, NULL, NULL, NULL, NULL, NULL, \
             {NO_PAYLOAD_COLUMNS}, NULL \
             FROM events ORDER BY sequence",
        ),
        7 => format!(
            "SELECT sequence, kind, step_index, workflow_fingerprint, component_name, \
             component_hash, input_value, output_value, artifact_hash, workflow_name, \
             duration_us, durability_required, component_count, artifact_backend, artifact_bytes, \
             planner_profile_id, planner_reason, NULL, NULL, NULL, NULL, start_index, start_input, \
             {NO_PAYLOAD_COLUMNS}, NULL \
             FROM events ORDER BY sequence",
        ),
        8 => "SELECT sequence, kind, step_index, workflow_fingerprint, component_name, \
             component_hash, input_value, output_value, artifact_hash, workflow_name, \
             duration_us, durability_required, component_count, artifact_backend, artifact_bytes, \
             planner_profile_id, planner_reason, NULL, NULL, NULL, NULL, start_index, start_input, \
             input_payload_kind, input_payload_inline, input_payload_hash, input_payload_bytes, \
             output_payload_kind, output_payload_inline, output_payload_hash, output_payload_bytes, \
             start_payload_kind, start_payload_inline, start_payload_hash, start_payload_bytes, NULL \
             FROM events ORDER BY sequence"
            .to_owned(),
        9 => "SELECT sequence, kind, step_index, workflow_fingerprint, component_name, \
             component_hash, input_value, output_value, artifact_hash, workflow_name, \
             duration_us, durability_required, component_count, artifact_backend, artifact_bytes, \
             planner_profile_id, planner_reason, planner_recompute_us, planner_checkpoint_us, \
             planner_checkpoint_bytes, planner_samples, start_index, start_input, \
             input_payload_kind, input_payload_inline, input_payload_hash, input_payload_bytes, \
             output_payload_kind, output_payload_inline, output_payload_hash, output_payload_bytes, \
             start_payload_kind, start_payload_inline, start_payload_hash, start_payload_bytes, NULL \
             FROM events ORDER BY sequence"
            .to_owned(),
        version if version == SCHEMA_VERSION => "SELECT sequence, kind, step_index, workflow_fingerprint, component_name, \
             component_hash, input_value, output_value, artifact_hash, workflow_name, \
             duration_us, durability_required, component_count, artifact_backend, artifact_bytes, \
             planner_profile_id, planner_reason, planner_recompute_us, planner_checkpoint_us, \
             planner_checkpoint_bytes, planner_samples, start_index, start_input, \
             input_payload_kind, input_payload_inline, input_payload_hash, input_payload_bytes, \
             output_payload_kind, output_payload_inline, output_payload_hash, output_payload_bytes, \
             start_payload_kind, start_payload_inline, start_payload_hash, start_payload_bytes, \
             planner_recorded_at_ms \
             FROM events ORDER BY sequence"
            .to_owned(),
        found => return Err(JournalError::UnsupportedSchema { found }),
    };
    Ok(query)
}

pub(crate) fn classify_read_error(source: rusqlite::Error) -> JournalError {
    if matches!(
        source.sqlite_error_code(),
        Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked)
    ) {
        let _ = source;
        JournalError::Busy
    } else {
        JournalError::Read { source }
    }
}

pub(crate) fn corrupt(sequence: i64, message: impl Into<String>) -> JournalError {
    JournalError::Corrupt {
        sequence,
        message: message.into(),
    }
}
