use rusqlite::Row;

use crate::journal::JournalError;

pub(crate) enum JournalEvent {
    WorkflowStarted {
        name: Option<String>,
        fingerprint: String,
        input: u32,
        component_count: Option<usize>,
    },
    ComponentStarted {
        index: usize,
        name: String,
        hash: String,
        input: u32,
        durable_after: Option<bool>,
    },
    ComponentCompleted {
        index: usize,
        output: u32,
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
        output: u32,
    },
    RecoveryTimed {
        duration_us: u64,
    },
}

struct StoredEvent {
    sequence: i64,
    kind: String,
    index: Option<i64>,
    fingerprint: Option<String>,
    name: Option<String>,
    hash: Option<String>,
    input: Option<i64>,
    output: Option<i64>,
    artifact_hash: Option<String>,
    workflow_name: Option<String>,
    duration_us: Option<i64>,
    durable_after: Option<i64>,
    component_count: Option<i64>,
    artifact_backend: Option<String>,
    artifact_bytes: Option<i64>,
}

pub(crate) fn decode_row(row: &Row<'_>) -> Result<(i64, JournalEvent), JournalError> {
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
    };
    let sequence = stored.sequence;
    decode_event(stored).map(|event| (sequence, event))
}

fn read<T: rusqlite::types::FromSql>(row: &Row<'_>, index: usize) -> Result<T, JournalError> {
    row.get(index)
        .map_err(|source| JournalError::Read { source })
}

fn decode_event(stored: StoredEvent) -> Result<JournalEvent, JournalError> {
    let StoredEvent {
        sequence,
        kind,
        index,
        fingerprint,
        name,
        hash,
        input,
        output,
        artifact_hash,
        workflow_name,
        duration_us,
        durable_after,
        component_count,
        artifact_backend,
        artifact_bytes,
    } = stored;
    match kind.as_str() {
        "workflow_started" => {
            absent(sequence, &index, "component index")?;
            absent(sequence, &name, "component name")?;
            absent(sequence, &hash, "component hash")?;
            absent(sequence, &output, "output")?;
            absent(sequence, &artifact_hash, "artifact hash")?;
            absent(sequence, &duration_us, "duration")?;
            absent(sequence, &durable_after, "durability")?;
            absent(sequence, &artifact_backend, "artifact backend")?;
            absent(sequence, &artifact_bytes, "artifact bytes")?;
            Ok(JournalEvent::WorkflowStarted {
                name: workflow_name,
                fingerprint: required(sequence, fingerprint, "workflow fingerprint")?,
                input: unsigned(sequence, input, "input")?,
                component_count: optional_index(sequence, component_count, "component count")?,
            })
        }
        "component_started" => {
            absent(sequence, &fingerprint, "workflow fingerprint")?;
            absent(sequence, &output, "output")?;
            absent(sequence, &artifact_hash, "artifact hash")?;
            absent(sequence, &workflow_name, "workflow name")?;
            absent(sequence, &duration_us, "duration")?;
            absent(sequence, &component_count, "component count")?;
            absent(sequence, &artifact_backend, "artifact backend")?;
            absent(sequence, &artifact_bytes, "artifact bytes")?;
            Ok(JournalEvent::ComponentStarted {
                index: component_index(sequence, index)?,
                name: required(sequence, name, "component name")?,
                hash: required(sequence, hash, "component hash")?,
                input: unsigned(sequence, input, "input")?,
                durable_after: optional_bool(sequence, durable_after, "durability")?,
            })
        }
        "component_completed" => {
            absent(sequence, &fingerprint, "workflow fingerprint")?;
            absent(sequence, &name, "component name")?;
            absent(sequence, &hash, "component hash")?;
            absent(sequence, &input, "input")?;
            absent(sequence, &artifact_hash, "artifact hash")?;
            absent(sequence, &workflow_name, "workflow name")?;
            absent(sequence, &durable_after, "durability")?;
            absent(sequence, &component_count, "component count")?;
            absent(sequence, &artifact_backend, "artifact backend")?;
            absent(sequence, &artifact_bytes, "artifact bytes")?;
            Ok(JournalEvent::ComponentCompleted {
                index: component_index(sequence, index)?,
                output: unsigned(sequence, output, "output")?,
                duration_us: optional_unsigned(sequence, duration_us, "duration")?,
            })
        }
        "checkpoint_created" => {
            absent(sequence, &fingerprint, "workflow fingerprint")?;
            absent(sequence, &name, "component name")?;
            absent(sequence, &hash, "component hash")?;
            absent(sequence, &input, "input")?;
            absent(sequence, &output, "output")?;
            absent(sequence, &workflow_name, "workflow name")?;
            absent(sequence, &durable_after, "durability")?;
            absent(sequence, &component_count, "component count")?;
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
            absent(sequence, &artifact_hash, "artifact hash")?;
            absent(sequence, &workflow_name, "workflow name")?;
            absent(sequence, &duration_us, "duration")?;
            absent(sequence, &durable_after, "durability")?;
            absent(sequence, &component_count, "component count")?;
            absent(sequence, &artifact_backend, "artifact backend")?;
            absent(sequence, &artifact_bytes, "artifact bytes")?;
            Ok(JournalEvent::WorkflowCompleted {
                output: unsigned(sequence, output, "output")?,
            })
        }
        "recovery_timed" => {
            absent(sequence, &index, "component index")?;
            absent(sequence, &fingerprint, "workflow fingerprint")?;
            absent(sequence, &name, "component name")?;
            absent(sequence, &hash, "component hash")?;
            absent(sequence, &input, "input")?;
            absent(sequence, &output, "output")?;
            absent(sequence, &artifact_hash, "artifact hash")?;
            absent(sequence, &workflow_name, "workflow name")?;
            absent(sequence, &durable_after, "durability")?;
            absent(sequence, &component_count, "component count")?;
            absent(sequence, &artifact_backend, "artifact backend")?;
            absent(sequence, &artifact_bytes, "artifact bytes")?;
            Ok(JournalEvent::RecoveryTimed {
                duration_us: unsigned_u64(sequence, duration_us, "duration")?,
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

fn unsigned(sequence: i64, value: Option<i64>, field: &str) -> Result<u32, JournalError> {
    let value = required(sequence, value, field)?;
    u32::try_from(value).map_err(|_| corrupt(sequence, format!("invalid {field}")))
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
