use rusqlite::Row;

use crate::journal::JournalError;
use crate::payload::{self, PayloadColumns};

use super::JournalEvent;

pub(super) struct StoredEvent {
    sequence: i64,
    kind: String,
    index: Option<i64>,
    fingerprint: Option<String>,
    name: Option<String>,
    hash: Option<String>,
    input: Option<i64>,
    input_payload: PayloadColumns,
    output: Option<i64>,
    output_payload: PayloadColumns,
    artifact_hash: Option<String>,
    workflow_name: Option<String>,
    duration_us: Option<i64>,
    durable_after: Option<i64>,
    component_count: Option<i64>,
    artifact_backend: Option<String>,
    artifact_bytes: Option<i64>,
    planner_profile_id: Option<String>,
    planner_reason: Option<String>,
    start_index: Option<i64>,
    start_input: Option<i64>,
    start_payload: PayloadColumns,
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
        start_index: read(row, 17)?,
        start_input: read(row, 18)?,
        input_payload: PayloadColumns {
            kind: read(row, 19)?,
            inline: read(row, 20)?,
            hash: read(row, 21)?,
            bytes: read(row, 22)?,
        },
        output_payload: PayloadColumns {
            kind: read(row, 23)?,
            inline: read(row, 24)?,
            hash: read(row, 25)?,
            bytes: read(row, 26)?,
        },
        start_payload: PayloadColumns {
            kind: read(row, 27)?,
            inline: read(row, 28)?,
            hash: read(row, 29)?,
            bytes: read(row, 30)?,
        },
    };
    let sequence = stored.sequence;
    Ok((sequence, stored))
}

fn read<T: rusqlite::types::FromSql>(row: &Row<'_>, index: usize) -> Result<T, JournalError> {
    row.get(index)
        .map_err(|source| JournalError::Read { source })
}

fn decode_payload(
    sequence: i64,
    field: &str,
    value: Option<i64>,
    columns: PayloadColumns,
) -> Result<payload::EventPayload, JournalError> {
    payload::decode_columns(sequence, field, value, columns)
}

pub(super) fn event(stored: StoredEvent) -> Result<JournalEvent, JournalError> {
    let StoredEvent {
        sequence,
        kind,
        index,
        fingerprint,
        name,
        hash,
        input,
        input_payload,
        output,
        output_payload,
        artifact_hash,
        workflow_name,
        duration_us,
        durable_after,
        component_count,
        artifact_backend,
        artifact_bytes,
        planner_profile_id,
        planner_reason,
        start_index,
        start_input,
        start_payload,
    } = stored;
    match kind.as_str() {
        "workflow_started" => {
            absent(sequence, &index, "component index")?;
            absent(sequence, &name, "component name")?;
            absent(sequence, &hash, "component hash")?;
            absent(sequence, &output, "output")?;
            payload::absent_columns(sequence, &output_payload, "output")?;
            absent(sequence, &artifact_hash, "artifact hash")?;
            absent(sequence, &duration_us, "duration")?;
            absent(sequence, &durable_after, "durability")?;
            absent(sequence, &artifact_backend, "artifact backend")?;
            absent(sequence, &artifact_bytes, "artifact bytes")?;
            absent(sequence, &planner_profile_id, "planner profile id")?;
            absent(sequence, &planner_reason, "planner reason")?;
            let start_input = if start_input.is_none() && start_payload.is_empty() {
                None
            } else {
                Some(decode_payload(
                    sequence,
                    "start input",
                    start_input,
                    start_payload,
                )?)
            };
            Ok(JournalEvent::WorkflowStarted {
                name: workflow_name,
                fingerprint: required(sequence, fingerprint, "workflow fingerprint")?,
                input: decode_payload(sequence, "input", input, input_payload)?,
                component_count: optional_index(sequence, component_count, "component count")?,
                start_index: optional_index(sequence, start_index, "start index")?,
                start_input,
            })
        }
        "component_started" => {
            absent(sequence, &fingerprint, "workflow fingerprint")?;
            absent(sequence, &output, "output")?;
            payload::absent_columns(sequence, &output_payload, "output")?;
            absent(sequence, &artifact_hash, "artifact hash")?;
            absent(sequence, &workflow_name, "workflow name")?;
            absent(sequence, &duration_us, "duration")?;
            absent(sequence, &component_count, "component count")?;
            absent(sequence, &artifact_backend, "artifact backend")?;
            absent(sequence, &artifact_bytes, "artifact bytes")?;
            absent(sequence, &planner_profile_id, "planner profile id")?;
            absent(sequence, &planner_reason, "planner reason")?;
            absent(sequence, &start_index, "start index")?;
            absent(sequence, &start_input, "start input")?;
            payload::absent_columns(sequence, &start_payload, "start input")?;
            Ok(JournalEvent::ComponentStarted {
                index: component_index(sequence, index)?,
                name: required(sequence, name, "component name")?,
                hash: required(sequence, hash, "component hash")?,
                input: decode_payload(sequence, "input", input, input_payload)?,
                durable_after: optional_bool(sequence, durable_after, "durability")?,
            })
        }
        "component_completed" => {
            absent(sequence, &fingerprint, "workflow fingerprint")?;
            absent(sequence, &name, "component name")?;
            absent(sequence, &hash, "component hash")?;
            absent(sequence, &input, "input")?;
            payload::absent_columns(sequence, &input_payload, "input")?;
            absent(sequence, &artifact_hash, "artifact hash")?;
            absent(sequence, &workflow_name, "workflow name")?;
            absent(sequence, &durable_after, "durability")?;
            absent(sequence, &component_count, "component count")?;
            absent(sequence, &artifact_backend, "artifact backend")?;
            absent(sequence, &artifact_bytes, "artifact bytes")?;
            absent(sequence, &planner_profile_id, "planner profile id")?;
            absent(sequence, &planner_reason, "planner reason")?;
            absent(sequence, &start_index, "start index")?;
            absent(sequence, &start_input, "start input")?;
            payload::absent_columns(sequence, &start_payload, "start input")?;
            Ok(JournalEvent::ComponentCompleted {
                index: component_index(sequence, index)?,
                output: decode_payload(sequence, "output", output, output_payload)?,
                duration_us: optional_unsigned(sequence, duration_us, "duration")?,
            })
        }
        "checkpoint_created" => {
            absent(sequence, &fingerprint, "workflow fingerprint")?;
            absent(sequence, &name, "component name")?;
            absent(sequence, &hash, "component hash")?;
            absent(sequence, &input, "input")?;
            payload::absent_columns(sequence, &input_payload, "input")?;
            absent(sequence, &output, "output")?;
            payload::absent_columns(sequence, &output_payload, "output")?;
            absent(sequence, &workflow_name, "workflow name")?;
            absent(sequence, &durable_after, "durability")?;
            absent(sequence, &component_count, "component count")?;
            absent(sequence, &planner_profile_id, "planner profile id")?;
            absent(sequence, &planner_reason, "planner reason")?;
            absent(sequence, &start_index, "start index")?;
            absent(sequence, &start_input, "start input")?;
            payload::absent_columns(sequence, &start_payload, "start input")?;
            Ok(JournalEvent::CheckpointCreated {
                index: component_index(sequence, index)?,
                hash: required(sequence, artifact_hash, "artifact hash")?,
                backend: artifact_backend,
                bytes: optional_unsigned(sequence, artifact_bytes, "artifact bytes")?,
                duration_us: optional_unsigned(sequence, duration_us, "duration")?,
            })
        }
        "workflow_completed" => {
            absent(sequence, &index, "component index")?;
            absent(sequence, &fingerprint, "workflow fingerprint")?;
            absent(sequence, &name, "component name")?;
            absent(sequence, &hash, "component hash")?;
            absent(sequence, &input, "input")?;
            payload::absent_columns(sequence, &input_payload, "input")?;
            absent(sequence, &artifact_hash, "artifact hash")?;
            absent(sequence, &workflow_name, "workflow name")?;
            absent(sequence, &duration_us, "duration")?;
            absent(sequence, &durable_after, "durability")?;
            absent(sequence, &component_count, "component count")?;
            absent(sequence, &artifact_backend, "artifact backend")?;
            absent(sequence, &artifact_bytes, "artifact bytes")?;
            absent(sequence, &planner_profile_id, "planner profile id")?;
            absent(sequence, &planner_reason, "planner reason")?;
            absent(sequence, &start_index, "start index")?;
            absent(sequence, &start_input, "start input")?;
            payload::absent_columns(sequence, &start_payload, "start input")?;
            Ok(JournalEvent::WorkflowCompleted {
                output: decode_payload(sequence, "output", output, output_payload)?,
            })
        }
        "recovery_timed" => {
            absent(sequence, &index, "component index")?;
            absent(sequence, &fingerprint, "workflow fingerprint")?;
            absent(sequence, &name, "component name")?;
            absent(sequence, &hash, "component hash")?;
            absent(sequence, &input, "input")?;
            payload::absent_columns(sequence, &input_payload, "input")?;
            absent(sequence, &output, "output")?;
            payload::absent_columns(sequence, &output_payload, "output")?;
            absent(sequence, &artifact_hash, "artifact hash")?;
            absent(sequence, &workflow_name, "workflow name")?;
            absent(sequence, &durable_after, "durability")?;
            absent(sequence, &component_count, "component count")?;
            absent(sequence, &artifact_backend, "artifact backend")?;
            absent(sequence, &artifact_bytes, "artifact bytes")?;
            absent(sequence, &planner_profile_id, "planner profile id")?;
            absent(sequence, &planner_reason, "planner reason")?;
            absent(sequence, &start_index, "start index")?;
            absent(sequence, &start_input, "start input")?;
            payload::absent_columns(sequence, &start_payload, "start input")?;
            Ok(JournalEvent::RecoveryTimed {
                duration_us: unsigned_u64(sequence, duration_us, "duration")?,
            })
        }
        "durability_planned" => {
            absent(sequence, &fingerprint, "workflow fingerprint")?;
            absent(sequence, &name, "component name")?;
            absent(sequence, &hash, "component hash")?;
            absent(sequence, &input, "input")?;
            payload::absent_columns(sequence, &input_payload, "input")?;
            absent(sequence, &output, "output")?;
            payload::absent_columns(sequence, &output_payload, "output")?;
            absent(sequence, &artifact_hash, "artifact hash")?;
            absent(sequence, &workflow_name, "workflow name")?;
            absent(sequence, &duration_us, "duration")?;
            absent(sequence, &component_count, "component count")?;
            absent(sequence, &artifact_backend, "artifact backend")?;
            absent(sequence, &artifact_bytes, "artifact bytes")?;
            absent(sequence, &start_index, "start index")?;
            absent(sequence, &start_input, "start input")?;
            payload::absent_columns(sequence, &start_payload, "start input")?;
            Ok(JournalEvent::DurabilityPlanned {
                index: component_index(sequence, index)?,
                required: required(
                    sequence,
                    optional_bool(sequence, durable_after, "durability")?,
                    "durability",
                )?,
                profile_id: required(sequence, planner_profile_id, "planner profile id")?,
                reason: required(sequence, planner_reason, "planner reason")?,
            })
        }
        _ => Err(corrupt(sequence, format!("unknown event kind `{kind}`"))),
    }
}

fn required<T>(sequence: i64, value: Option<T>, field: &str) -> Result<T, JournalError> {
    value.ok_or_else(|| corrupt(sequence, format!("missing {field}")))
}

fn absent<T>(sequence: i64, value: &Option<T>, field: &str) -> Result<(), JournalError> {
    if value.is_some() {
        return Err(corrupt(sequence, format!("unexpected {field}")));
    }
    Ok(())
}

fn unsigned_u64(sequence: i64, value: Option<i64>, field: &str) -> Result<u64, JournalError> {
    let value = required(sequence, value, field)?;
    u64::try_from(value).map_err(|_| corrupt(sequence, format!("invalid {field}")))
}

fn optional_unsigned(
    sequence: i64,
    value: Option<i64>,
    field: &str,
) -> Result<Option<u64>, JournalError> {
    value
        .map(|value| {
            u64::try_from(value).map_err(|_| corrupt(sequence, format!("invalid {field}")))
        })
        .transpose()
}

fn optional_bool(
    sequence: i64,
    value: Option<i64>,
    field: &str,
) -> Result<Option<bool>, JournalError> {
    value
        .map(|value| match value {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(corrupt(sequence, format!("invalid {field}"))),
        })
        .transpose()
}

fn component_index(sequence: i64, value: Option<i64>) -> Result<usize, JournalError> {
    let value = required(sequence, value, "component index")?;
    usize::try_from(value).map_err(|_| corrupt(sequence, "invalid component index"))
}

fn optional_index(
    sequence: i64,
    value: Option<i64>,
    field: &str,
) -> Result<Option<usize>, JournalError> {
    value
        .map(|value| {
            usize::try_from(value).map_err(|_| corrupt(sequence, format!("invalid {field}")))
        })
        .transpose()
}

fn corrupt(sequence: i64, message: impl Into<String>) -> JournalError {
    JournalError::Corrupt {
        sequence,
        message: message.into(),
    }
}
