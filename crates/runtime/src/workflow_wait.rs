use std::{path::Path, time::Duration};

use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use thiserror::Error;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DurableWait {
    Timer { due_ms: u64 },
    Signal { name: String },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkflowWaitState {
    pub after_step: usize,
    pub wait: DurableWait,
    pub completed: bool,
}

#[derive(Debug, Error)]
pub enum WorkflowWaitError {
    #[error("failed to access durable workflow wait state")]
    Database {
        #[source]
        source: rusqlite::Error,
    },
    #[error("durable workflow wait state is invalid")]
    Invalid,
    #[error("durable workflow wait does not match the recorded boundary")]
    Conflict,
}

pub fn record_workflow_wait(
    path: &Path,
    after_step: usize,
    wait: &DurableWait,
) -> Result<WorkflowWaitState, WorkflowWaitError> {
    let connection = open(path)?;
    create_table(&connection)?;
    let (kind, value) = encode(wait);
    connection
        .execute(
            "INSERT OR IGNORE INTO workflow_wait(id, after_step, kind, value, completed) VALUES (1, ?1, ?2, ?3, 0)",
            params![as_i64(after_step)?, kind, value],
        )
        .map_err(database)?;
    let state = read(&connection)?.ok_or(WorkflowWaitError::Invalid)?;
    if state.after_step != after_step || &state.wait != wait {
        return Err(WorkflowWaitError::Conflict);
    }
    Ok(state)
}

pub fn inspect_workflow_wait(path: &Path) -> Result<Option<WorkflowWaitState>, WorkflowWaitError> {
    if !path.exists() {
        return Ok(None);
    }
    let connection =
        Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(database)?;
    let exists = connection
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name='workflow_wait'",
            [],
            |_| Ok(()),
        )
        .optional()
        .map_err(database)?
        .is_some();
    if exists { read(&connection) } else { Ok(None) }
}

pub fn complete_workflow_wait(path: &Path, after_step: usize) -> Result<(), WorkflowWaitError> {
    let connection = open(path)?;
    create_table(&connection)?;
    let changed = connection
        .execute(
            "UPDATE workflow_wait SET completed=1 WHERE id=1 AND after_step=?1",
            [as_i64(after_step)?],
        )
        .map_err(database)?;
    if changed == 0 {
        return Err(WorkflowWaitError::Conflict);
    }
    Ok(())
}

fn open(path: &Path) -> Result<Connection, WorkflowWaitError> {
    let connection = Connection::open(path).map_err(database)?;
    connection
        .busy_timeout(Duration::from_secs(2))
        .map_err(database)?;
    Ok(connection)
}

fn create_table(connection: &Connection) -> Result<(), WorkflowWaitError> {
    connection
        .execute_batch("CREATE TABLE IF NOT EXISTS workflow_wait(id INTEGER PRIMARY KEY CHECK(id=1), after_step INTEGER NOT NULL, kind TEXT NOT NULL, value TEXT NOT NULL, completed INTEGER NOT NULL CHECK(completed IN (0,1)))")
        .map_err(database)
}

fn read(connection: &Connection) -> Result<Option<WorkflowWaitState>, WorkflowWaitError> {
    connection
        .query_row(
            "SELECT after_step, kind, value, completed FROM workflow_wait WHERE id=1",
            [],
            |row| {
                let after_step = row.get::<_, i64>(0)?;
                let kind = row.get::<_, String>(1)?;
                let value = row.get::<_, String>(2)?;
                let completed = row.get::<_, i64>(3)?;
                Ok((after_step, kind, value, completed))
            },
        )
        .optional()
        .map_err(database)?
        .map(decode)
        .transpose()
}

fn encode(wait: &DurableWait) -> (&'static str, String) {
    match wait {
        DurableWait::Timer { due_ms } => ("timer", due_ms.to_string()),
        DurableWait::Signal { name } => ("signal", name.clone()),
    }
}

fn decode(
    (after_step, kind, value, completed): (i64, String, String, i64),
) -> Result<WorkflowWaitState, WorkflowWaitError> {
    let after_step = usize::try_from(after_step).map_err(|_| WorkflowWaitError::Invalid)?;
    let wait = match kind.as_str() {
        "timer" => DurableWait::Timer {
            due_ms: value.parse().map_err(|_| WorkflowWaitError::Invalid)?,
        },
        "signal" if !value.is_empty() => DurableWait::Signal { name: value },
        _ => return Err(WorkflowWaitError::Invalid),
    };
    Ok(WorkflowWaitState {
        after_step,
        wait,
        completed: match completed {
            0 => false,
            1 => true,
            _ => return Err(WorkflowWaitError::Invalid),
        },
    })
}

fn as_i64(value: usize) -> Result<i64, WorkflowWaitError> {
    i64::try_from(value).map_err(|_| WorkflowWaitError::Invalid)
}

fn database(source: rusqlite::Error) -> WorkflowWaitError {
    WorkflowWaitError::Database { source }
}
