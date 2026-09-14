use std::path::Path;

use rusqlite::{Connection, OpenFlags};

use crate::{
    inspection::{READ_TIMEOUT, event_query},
    journal::JournalError,
    journal_event::{JournalEvent, decode_row},
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CellEvent {
    pub sequence: i64,
    pub summary: String,
}

pub fn inspect_events(
    path: impl AsRef<Path>,
    after: i64,
    limit: usize,
) -> Result<Vec<CellEvent>, JournalError> {
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|source| JournalError::Open { source })?;
    connection
        .busy_timeout(READ_TIMEOUT)
        .map_err(|source| JournalError::Configure { source })?;
    let version = connection
        .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
        .map_err(super::inspection::classify_read_error)?;
    let mut statement = connection
        .prepare(event_query(version)?)
        .map_err(super::inspection::classify_read_error)?;
    let mut rows = statement
        .query([])
        .map_err(super::inspection::classify_read_error)?;
    let mut events = Vec::new();
    while let Some(row) = rows
        .next()
        .map_err(super::inspection::classify_read_error)?
    {
        let (sequence, event) = decode_row(row)?;
        if sequence <= after {
            continue;
        }
        events.push(CellEvent {
            sequence,
            summary: event_summary(event),
        });
        if events.len() == limit {
            break;
        }
    }
    Ok(events)
}

fn event_summary(event: JournalEvent) -> String {
    match event {
        JournalEvent::WorkflowStarted { name, .. } => {
            format!("{} started", name.unwrap_or_else(|| "workflow".to_owned()))
        }
        JournalEvent::ComponentStarted { name, .. } => format!("{name} started"),
        JournalEvent::ComponentCompleted { index, .. } => format!("step {} completed", index + 1),
        JournalEvent::CheckpointCreated { index, .. } => {
            format!("checkpoint saved after step {}", index + 1)
        }
        JournalEvent::WorkflowCompleted { .. } => "workflow completed".to_owned(),
        JournalEvent::RecoveryTimed { duration_us } => format!("recovered in {duration_us}µs"),
    }
}
