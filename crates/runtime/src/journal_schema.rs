use rusqlite::{Connection, OptionalExtension};

use crate::journal::JournalError;

pub(crate) const SCHEMA_VERSION: i64 = 5;

pub(crate) fn initialize(connection: &Connection) -> Result<(), JournalError> {
    let version = connection
        .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
        .map_err(|source| JournalError::Read { source })?;
    if version == SCHEMA_VERSION {
        return Ok(());
    }
    if version == 1 {
        return connection
            .execute_batch(
                "BEGIN IMMEDIATE;
                ALTER TABLE events ADD COLUMN artifact_hash TEXT;
                ALTER TABLE events ADD COLUMN workflow_name TEXT;
                ALTER TABLE events ADD COLUMN duration_us INTEGER;
                ALTER TABLE events ADD COLUMN durability_required INTEGER;
                ALTER TABLE events ADD COLUMN component_count INTEGER;
                ALTER TABLE events ADD COLUMN artifact_backend TEXT;
                ALTER TABLE events ADD COLUMN artifact_bytes INTEGER;
                PRAGMA user_version = 5;
                COMMIT;",
            )
            .map_err(|source| JournalError::Configure { source });
    }
    if version == 2 {
        return connection
            .execute_batch(
                "BEGIN IMMEDIATE;
                ALTER TABLE events ADD COLUMN workflow_name TEXT;
                ALTER TABLE events ADD COLUMN duration_us INTEGER;
                ALTER TABLE events ADD COLUMN durability_required INTEGER;
                ALTER TABLE events ADD COLUMN component_count INTEGER;
                ALTER TABLE events ADD COLUMN artifact_backend TEXT;
                ALTER TABLE events ADD COLUMN artifact_bytes INTEGER;
                PRAGMA user_version = 5;
                COMMIT;",
            )
            .map_err(|source| JournalError::Configure { source });
    }
    if version == 3 {
        return connection
            .execute_batch(
                "BEGIN IMMEDIATE;
                ALTER TABLE events ADD COLUMN artifact_backend TEXT;
                ALTER TABLE events ADD COLUMN artifact_bytes INTEGER;
                PRAGMA user_version = 5;
                COMMIT;",
            )
            .map_err(|source| JournalError::Configure { source });
    }
    if version == 4 {
        return connection
            .execute_batch(
                "BEGIN IMMEDIATE;
                ALTER TABLE events ADD COLUMN artifact_bytes INTEGER;
                PRAGMA user_version = 5;
                COMMIT;",
            )
            .map_err(|source| JournalError::Configure { source });
    }
    if version != 0 || has_schema(connection)? {
        return Err(JournalError::UnsupportedSchema { found: version });
    }
    connection
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
                artifact_hash TEXT,
                workflow_name TEXT,
                duration_us INTEGER,
                durability_required INTEGER,
                component_count INTEGER,
                artifact_backend TEXT,
                artifact_bytes INTEGER
            ) STRICT;
            PRAGMA user_version = 5;
            COMMIT;",
        )
        .map_err(|source| JournalError::Configure { source })
}

fn has_schema(connection: &Connection) -> Result<bool, JournalError> {
    connection
        .query_row(
            "SELECT 1 FROM sqlite_schema WHERE type = 'table' LIMIT 1",
            [],
            |_| Ok(()),
        )
        .optional()
        .map(|row| row.is_some())
        .map_err(|source| JournalError::Read { source })
}
