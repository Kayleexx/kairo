use rusqlite::Row;

use crate::journal::JournalError;

pub(crate) enum JournalEvent {
    WorkflowStarted {
        fingerprint: String,
        input: u32,
    },
    ComponentStarted {
        index: usize,
        name: String,
        hash: String,
        input: u32,
    },
    ComponentCompleted {
        index: usize,
        output: u32,
    },
    WorkflowCompleted {
        output: u32,
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
    } = stored;
    match kind.as_str() {
        "workflow_started" => {
            absent(sequence, &index, "component index")?;
            absent(sequence, &name, "component name")?;
            absent(sequence, &hash, "component hash")?;
            absent(sequence, &output, "output")?;
            Ok(JournalEvent::WorkflowStarted {
                fingerprint: required(sequence, fingerprint, "workflow fingerprint")?,
                input: unsigned(sequence, input, "input")?,
            })
        }
        "component_started" => {
            absent(sequence, &fingerprint, "workflow fingerprint")?;
            absent(sequence, &output, "output")?;
            Ok(JournalEvent::ComponentStarted {
                index: component_index(sequence, index)?,
                name: required(sequence, name, "component name")?,
                hash: required(sequence, hash, "component hash")?,
                input: unsigned(sequence, input, "input")?,
            })
        }
        "component_completed" => {
            absent(sequence, &fingerprint, "workflow fingerprint")?;
            absent(sequence, &name, "component name")?;
            absent(sequence, &hash, "component hash")?;
            absent(sequence, &input, "input")?;
            Ok(JournalEvent::ComponentCompleted {
                index: component_index(sequence, index)?,
                output: unsigned(sequence, output, "output")?,
            })
        }
        "workflow_completed" => {
            absent(sequence, &index, "component index")?;
            absent(sequence, &fingerprint, "workflow fingerprint")?;
            absent(sequence, &name, "component name")?;
            absent(sequence, &hash, "component hash")?;
            absent(sequence, &input, "input")?;
            Ok(JournalEvent::WorkflowCompleted {
                output: unsigned(sequence, output, "output")?,
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

fn component_index(sequence: i64, value: Option<i64>) -> Result<usize, JournalError> {
    let value = required(sequence, value, "component index")?;
    usize::try_from(value).map_err(|_| corrupt(sequence, "invalid component index"))
}

fn corrupt(sequence: i64, message: impl Into<String>) -> JournalError {
    JournalError::Corrupt {
        sequence,
        message: message.into(),
    }
}
