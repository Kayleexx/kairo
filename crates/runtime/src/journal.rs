use std::{fs, path::Path, time::Duration};

use rusqlite::{Connection, ErrorCode, OptionalExtension, params};
use thiserror::Error;

use crate::journal_event::{JournalEvent, decode_row};

const SCHEMA_VERSION: i64 = 2;
const LOCK_TIMEOUT: Duration = Duration::from_millis(250);

#[derive(Debug, Error)]
pub enum JournalError {
    #[error("failed to create state directory `{path}`")]
    CreateDirectory {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to open the journal")]
    Open {
        #[source]
        source: rusqlite::Error,
    },
    #[error("the journal is already in use")]
    Busy {
        #[source]
        source: rusqlite::Error,
    },
    #[error("failed to configure the journal")]
    Configure {
        #[source]
        source: rusqlite::Error,
    },
    #[error("journal schema version {found} is unsupported")]
    UnsupportedSchema { found: i64 },
    #[error("failed to read the journal")]
    Read {
        #[source]
        source: rusqlite::Error,
    },
    #[error("failed to append an execution event")]
    Write {
        #[source]
        source: rusqlite::Error,
    },
    #[error("journal event {sequence} is invalid: {message}")]
    Corrupt { sequence: i64, message: String },
    #[error(
        "the journal belongs to a different workflow; use a new `--state` path or archive the existing journal"
    )]
    WorkflowChanged,
    #[error("invalid Cell state: {message}")]
    InvalidState { message: String },
}

pub(crate) struct Journal {
    connection: Connection,
}

impl Journal {
    pub(crate) fn open(path: &Path) -> Result<Self, JournalError> {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent).map_err(|source| JournalError::CreateDirectory {
                path: parent.to_path_buf(),
                source,
            })?;
        }

        let connection = Connection::open(path).map_err(|source| JournalError::Open { source })?;
        connection
            .busy_timeout(LOCK_TIMEOUT)
            .map_err(|source| JournalError::Configure { source })?;
        connection
            .pragma_update(None, "locking_mode", "EXCLUSIVE")
            .map_err(|source| JournalError::Configure { source })?;
        connection
            .execute_batch("BEGIN EXCLUSIVE; COMMIT;")
            .map_err(classify_lock_error)?;
        connection
            .pragma_update(None, "synchronous", "FULL")
            .map_err(|source| JournalError::Configure { source })?;

        let journal = Self { connection };
        journal.initialize()?;
        Ok(journal)
    }

    pub(crate) fn append(&self, event: &JournalEvent) -> Result<(), JournalError> {
        let (kind, index, fingerprint, name, hash, input, output, artifact_hash) = match event {
            JournalEvent::WorkflowStarted { fingerprint, input } => (
                "workflow_started",
                None,
                Some(fingerprint.as_str()),
                None,
                None,
                Some(i64::from(*input)),
                None,
                None,
            ),
            JournalEvent::ComponentStarted {
                index,
                name,
                hash,
                input,
            } => (
                "component_started",
                Some(index_value(*index)?),
                None,
                Some(name.as_str()),
                Some(hash.as_str()),
                Some(i64::from(*input)),
                None,
                None,
            ),
            JournalEvent::ComponentCompleted { index, output } => (
                "component_completed",
                Some(index_value(*index)?),
                None,
                None,
                None,
                None,
                Some(i64::from(*output)),
                None,
            ),
            JournalEvent::CheckpointCreated { index, hash } => (
                "checkpoint_created",
                Some(index_value(*index)?),
                None,
                None,
                None,
                None,
                None,
                Some(hash.as_str()),
            ),
            JournalEvent::WorkflowCompleted { output } => (
                "workflow_completed",
                None,
                None,
                None,
                None,
                None,
                Some(i64::from(*output)),
                None,
            ),
        };
        self.connection
            .execute(
                "INSERT INTO events(\
                    kind, step_index, workflow_fingerprint, component_name, component_hash, \
                    input_value, output_value, artifact_hash\
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    kind,
                    index,
                    fingerprint,
                    name,
                    hash,
                    input,
                    output,
                    artifact_hash
                ],
            )
            .map_err(|source| JournalError::Write { source })?;
        Ok(())
    }

    pub(crate) fn replay(
        &self,
        mut visit: impl FnMut(i64, JournalEvent) -> Result<(), JournalError>,
    ) -> Result<bool, JournalError> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT sequence, kind, step_index, workflow_fingerprint, component_name, \
                        component_hash, input_value, output_value, artifact_hash \
                 FROM events ORDER BY sequence",
            )
            .map_err(|source| JournalError::Read { source })?;
        let mut rows = statement
            .query([])
            .map_err(|source| JournalError::Read { source })?;
        let mut found = false;
        let mut expected_sequence = 1_i64;
        while let Some(row) = rows
            .next()
            .map_err(|source| JournalError::Read { source })?
        {
            found = true;
            let (sequence, event) = decode_row(row)?;
            if sequence != expected_sequence {
                return Err(corrupt(sequence, "event sequence is not contiguous"));
            }
            expected_sequence = expected_sequence
                .checked_add(1)
                .ok_or_else(|| corrupt(sequence, "event sequence overflow"))?;
            visit(sequence, event)?;
        }
        Ok(found)
    }

    fn initialize(&self) -> Result<(), JournalError> {
        let version = self
            .connection
            .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .map_err(|source| JournalError::Read { source })?;
        if version == SCHEMA_VERSION {
            return Ok(());
        }
        if version == 1 {
            return self
                .connection
                .execute_batch(
                    "BEGIN IMMEDIATE;
                    ALTER TABLE events ADD COLUMN artifact_hash TEXT;
                    PRAGMA user_version = 2;
                    COMMIT;",
                )
                .map_err(|source| JournalError::Configure { source });
        }
        if version != 0 || self.has_schema()? {
            return Err(JournalError::UnsupportedSchema { found: version });
        }
        self.connection
            .execute_batch(
                "BEGIN IMMEDIATE;
                CREATE TABLE events (
                    sequence INTEGER PRIMARY KEY,
                    kind TEXT NOT NULL,
                    step_index INTEGER,
                    workflow_fingerprint TEXT,
                    component_name TEXT,
                    component_hash TEXT,
                    input_value INTEGER,
                    output_value INTEGER,
                    artifact_hash TEXT
                ) STRICT;
                PRAGMA user_version = 2;
                COMMIT;",
            )
            .map_err(|source| JournalError::Configure { source })
    }

    fn has_schema(&self) -> Result<bool, JournalError> {
        self.connection
            .query_row(
                "SELECT 1 FROM sqlite_schema WHERE type = 'table' LIMIT 1",
                [],
                |_| Ok(()),
            )
            .optional()
            .map(|row| row.is_some())
            .map_err(|source| JournalError::Read { source })
    }
}

fn index_value(index: usize) -> Result<i64, JournalError> {
    i64::try_from(index).map_err(|_| JournalError::InvalidState {
        message: "component index is too large".to_owned(),
    })
}

fn corrupt(sequence: i64, message: impl Into<String>) -> JournalError {
    JournalError::Corrupt {
        sequence,
        message: message.into(),
    }
}

fn classify_lock_error(source: rusqlite::Error) -> JournalError {
    if matches!(
        source.sqlite_error_code(),
        Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked)
    ) {
        JournalError::Busy { source }
    } else {
        JournalError::Configure { source }
    }
}
