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
    let mut statement = connection.prepare(query).map_err(classify_read_error)?;
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

pub(crate) fn event_query(version: i64) -> Result<&'static str, JournalError> {
    match version {
        1 => Ok(
            "SELECT sequence, kind, step_index, workflow_fingerprint, component_name, \
                 component_hash, input_value, output_value, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL \
                 FROM events ORDER BY sequence",
        ),
        2 => Ok(
            "SELECT sequence, kind, step_index, workflow_fingerprint, component_name, \
                 component_hash, input_value, output_value, artifact_hash, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL \
                 FROM events ORDER BY sequence",
        ),
        3 => Ok(
            "SELECT sequence, kind, step_index, workflow_fingerprint, component_name, \
             component_hash, input_value, output_value, artifact_hash, workflow_name, \
             duration_us, durability_required, component_count, NULL, NULL, NULL, NULL, NULL, NULL FROM events ORDER BY sequence",
        ),
        4 => Ok(
            "SELECT sequence, kind, step_index, workflow_fingerprint, component_name, \
             component_hash, input_value, output_value, artifact_hash, workflow_name, \
             duration_us, durability_required, component_count, artifact_backend, NULL, NULL, NULL, NULL, NULL \
             FROM events ORDER BY sequence",
        ),
        5 => Ok(
            "SELECT sequence, kind, step_index, workflow_fingerprint, component_name, \
             component_hash, input_value, output_value, artifact_hash, workflow_name, \
             duration_us, durability_required, component_count, artifact_backend, artifact_bytes, NULL, NULL, NULL, NULL \
             FROM events ORDER BY sequence",
        ),
        6 => Ok(
            "SELECT sequence, kind, step_index, workflow_fingerprint, component_name, \
             component_hash, input_value, output_value, artifact_hash, workflow_name, \
             duration_us, durability_required, component_count, artifact_backend, artifact_bytes, \
             planner_profile_id, planner_reason, NULL, NULL \
             FROM events ORDER BY sequence",
        ),
        version if version == SCHEMA_VERSION => Ok(
            "SELECT sequence, kind, step_index, workflow_fingerprint, component_name, \
             component_hash, input_value, output_value, artifact_hash, workflow_name, \
             duration_us, durability_required, component_count, artifact_backend, artifact_bytes, \
             planner_profile_id, planner_reason, start_index, start_input \
             FROM events ORDER BY sequence",
        ),
        found => Err(JournalError::UnsupportedSchema { found }),
    }
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
