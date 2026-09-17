use rusqlite::Row;

use crate::journal::JournalError;
use crate::payload::{self, PayloadColumns};

pub(super) struct StoredEvent {
    pub(super) sequence: i64,
    pub(super) kind: String,
    pub(super) index: Option<i64>,
    pub(super) fingerprint: Option<String>,
    pub(super) name: Option<String>,
    pub(super) hash: Option<String>,
    pub(super) input: Option<i64>,
    pub(super) input_payload: PayloadColumns,
    pub(super) output: Option<i64>,
    pub(super) output_payload: PayloadColumns,
    pub(super) artifact_hash: Option<String>,
    pub(super) workflow_name: Option<String>,
    pub(super) duration_us: Option<i64>,
    pub(super) durable_after: Option<i64>,
    pub(super) component_count: Option<i64>,
    pub(super) artifact_backend: Option<String>,
    pub(super) artifact_bytes: Option<i64>,
    pub(super) planner_profile_id: Option<String>,
    pub(super) planner_reason: Option<String>,
    pub(super) planner_recompute_us: Option<i64>,
    pub(super) planner_checkpoint_us: Option<i64>,
    pub(super) planner_checkpoint_bytes: Option<i64>,
    pub(super) planner_samples: Option<i64>,
    pub(super) planner_recorded_at_ms: Option<i64>,
    pub(super) start_index: Option<i64>,
    pub(super) start_input: Option<i64>,
    pub(super) start_payload: PayloadColumns,
}

pub(super) fn from_row(row: &Row<'_>) -> Result<(i64, StoredEvent), JournalError> {
    let stored = StoredEvent {
        sequence: read(row, 0)?,
        kind: read(row, 1)?,
        index: read(row, 2)?,
        fingerprint: read(row, 3)?,
        name: read(row, 4)?,
        hash: read(row, 5)?,
        input: read(row, 6)?,
        output: read(row, 7)?,
        artifact_hash: read(row, 8)?,
        workflow_name: read(row, 9)?,
        duration_us: read(row, 10)?,
        durable_after: read(row, 11)?,
        component_count: read(row, 12)?,
        artifact_backend: read(row, 13)?,
        artifact_bytes: read(row, 14)?,
        planner_profile_id: read(row, 15)?,
        planner_reason: read(row, 16)?,
        planner_recompute_us: read(row, 17)?,
        planner_checkpoint_us: read(row, 18)?,
        planner_checkpoint_bytes: read(row, 19)?,
        planner_samples: read(row, 20)?,
        start_index: read(row, 21)?,
        start_input: read(row, 22)?,
        input_payload: PayloadColumns {
            kind: read(row, 23)?,
            inline: read(row, 24)?,
            hash: read(row, 25)?,
            bytes: read(row, 26)?,
        },
        output_payload: PayloadColumns {
            kind: read(row, 27)?,
            inline: read(row, 28)?,
            hash: read(row, 29)?,
            bytes: read(row, 30)?,
        },
        start_payload: PayloadColumns {
            kind: read(row, 31)?,
            inline: read(row, 32)?,
            hash: read(row, 33)?,
            bytes: read(row, 34)?,
        },
        planner_recorded_at_ms: read(row, 35)?,
    };
    let sequence = stored.sequence;
    Ok((sequence, stored))
}

fn read<T: rusqlite::types::FromSql>(row: &Row<'_>, index: usize) -> Result<T, JournalError> {
    row.get(index)
        .map_err(|source| JournalError::Read { source })
}

pub(super) fn decode_payload(
    sequence: i64,
    field: &str,
    value: Option<i64>,
    columns: PayloadColumns,
) -> Result<payload::EventPayload, JournalError> {
    payload::decode_columns(sequence, field, value, columns)
}
