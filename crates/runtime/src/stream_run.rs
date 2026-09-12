use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use thiserror::Error;

use crate::{StreamMetrics, StreamValue, WorkflowOutputArtifact};

const MAX_ERROR_CHARS: usize = 1024;

#[derive(Debug, Error)]
pub enum StreamRunError {
    #[error("failed to create stream run directory `{path}`")]
    CreateDirectory {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to open stream run state")]
    Open {
        #[source]
        source: rusqlite::Error,
    },
    #[error("failed to configure stream run state")]
    Configure {
        #[source]
        source: rusqlite::Error,
    },
    #[error("failed to write stream run state")]
    Write {
        #[source]
        source: rusqlite::Error,
    },
    #[error("failed to read stream run state")]
    Read {
        #[source]
        source: rusqlite::Error,
    },
    #[error("stream run state is invalid")]
    Invalid,
    #[error("stream run state `{path}` already exists; choose a different run name")]
    AlreadyExists { path: PathBuf },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StreamRunStatus {
    Running,
    Completed,
    Failed(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StreamRunInspection {
    pub workflow: String,
    pub input: String,
    pub input_source: Option<String>,
    pub input_hash: Option<String>,
    pub input_accepts: Vec<String>,
    pub status: StreamRunStatus,
    pub steps: Vec<String>,
    pub duration_us: Option<u64>,
    pub high: Option<u64>,
    pub low: Option<u32>,
    pub high_label: Option<String>,
    pub low_label: Option<String>,
    pub metrics: Option<StreamMetrics>,
    pub values: Vec<StreamValue>,
    pub outputs: Vec<WorkflowOutputArtifact>,
}

pub struct StreamRun {
    connection: Connection,
}

impl StreamRun {
    pub fn start(
        path: &Path,
        workflow: &str,
        input: &Path,
        steps: &[String],
        labels: Option<(&str, &str)>,
    ) -> Result<Self, StreamRunError> {
        Self::start_with_provenance(
            path,
            workflow,
            &input.display().to_string(),
            "local",
            &[],
            steps,
            labels,
        )
    }

    pub fn start_with_provenance(
        path: &Path,
        workflow: &str,
        input: &str,
        input_source: &str,
        accepts: &[String],
        steps: &[String],
        labels: Option<(&str, &str)>,
    ) -> Result<Self, StreamRunError> {
        if path.exists() {
            return Err(StreamRunError::AlreadyExists {
                path: path.to_path_buf(),
            });
        }
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent).map_err(|source| StreamRunError::CreateDirectory {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        let mut connection =
            Connection::open(path).map_err(|source| StreamRunError::Open { source })?;
        connection
            .pragma_update(None, "journal_mode", "WAL")
            .map_err(|source| StreamRunError::Configure { source })?;
        connection
            .pragma_update(None, "synchronous", "FULL")
            .map_err(|source| StreamRunError::Configure { source })?;
        connection.execute_batch("CREATE TABLE IF NOT EXISTS stream_run(id INTEGER PRIMARY KEY CHECK(id=1), workflow TEXT NOT NULL, input TEXT NOT NULL, input_source TEXT, input_hash TEXT, input_accepts TEXT, status TEXT NOT NULL, error TEXT, duration_us INTEGER, high INTEGER, low INTEGER, high_label TEXT, low_label TEXT, source_bytes INTEGER, consumed_bytes INTEGER, largest_batch_bytes INTEGER, materialized_bytes INTEGER); CREATE TABLE IF NOT EXISTS stream_steps(step_index INTEGER PRIMARY KEY, name TEXT NOT NULL); CREATE TABLE IF NOT EXISTS stream_values(value_index INTEGER PRIMARY KEY, name TEXT NOT NULL, value INTEGER NOT NULL); CREATE TABLE IF NOT EXISTS stream_outputs(output_index INTEGER PRIMARY KEY, filename TEXT NOT NULL, content_type TEXT NOT NULL, bytes INTEGER NOT NULL, hash TEXT NOT NULL, backend TEXT NOT NULL, reference TEXT NOT NULL, exported_path TEXT);").map_err(|source| StreamRunError::Write { source })?;
        let transaction = connection
            .transaction()
            .map_err(|source| StreamRunError::Write { source })?;
        let (high_label, low_label) =
            labels.map_or((None, None), |(high, low)| (Some(high), Some(low)));
        transaction.execute("INSERT OR REPLACE INTO stream_run(id, workflow, input, input_source, input_accepts, status, high_label, low_label) VALUES (1, ?1, ?2, ?3, ?4, 'running', ?5, ?6)", params![workflow, input, input_source, accepts.join(","), high_label, low_label]).map_err(|source| StreamRunError::Write { source })?;
        transaction
            .execute("DELETE FROM stream_steps", [])
            .map_err(|source| StreamRunError::Write { source })?;
        for (index, name) in steps.iter().enumerate() {
            transaction
                .execute(
                    "INSERT INTO stream_steps(step_index, name) VALUES (?1, ?2)",
                    params![
                        i64::try_from(index).map_err(|_| StreamRunError::Invalid)?,
                        name
                    ],
                )
                .map_err(|source| StreamRunError::Write { source })?;
        }
        transaction
            .commit()
            .map_err(|source| StreamRunError::Write { source })?;
        Ok(Self { connection })
    }

    pub fn complete(
        &mut self,
        duration: Duration,
        high: u64,
        low: u32,
        metrics: StreamMetrics,
    ) -> Result<(), StreamRunError> {
        self.complete_with_hash(duration, high, low, metrics, None)
    }

    pub fn complete_with_hash(
        &mut self,
        duration: Duration,
        high: u64,
        low: u32,
        metrics: StreamMetrics,
        input_hash: Option<&str>,
    ) -> Result<(), StreamRunError> {
        self.complete_with_values(duration, high, low, metrics, input_hash, &[])
    }

    pub fn complete_with_values(
        &mut self,
        duration: Duration,
        high: u64,
        low: u32,
        metrics: StreamMetrics,
        input_hash: Option<&str>,
        values: &[StreamValue],
    ) -> Result<(), StreamRunError> {
        self.complete_with_outputs(duration, high, low, metrics, input_hash, values, &[])
    }

    #[allow(clippy::too_many_arguments)]
    pub fn complete_with_outputs(
        &mut self,
        duration: Duration,
        high: u64,
        low: u32,
        metrics: StreamMetrics,
        input_hash: Option<&str>,
        values: &[StreamValue],
        outputs: &[WorkflowOutputArtifact],
    ) -> Result<(), StreamRunError> {
        let consumed = as_i64(u128::from(metrics.consumed_bytes))?;
        let transaction = self
            .connection
            .transaction()
            .map_err(|source| StreamRunError::Write { source })?;
        transaction.execute("UPDATE stream_run SET status='completed', error=NULL, duration_us=?1, high=?2, low=?3, source_bytes=?4, consumed_bytes=?5, largest_batch_bytes=?6, materialized_bytes=?7, input_hash=?8 WHERE id=1", params![as_i64(duration.as_micros())?, as_i64(u128::from(high))?, i64::from(low), as_i64(u128::from(metrics.source_bytes))?, consumed, i64::try_from(metrics.largest_batch_bytes).map_err(|_| StreamRunError::Invalid)?, as_i64(u128::from(metrics.materialized_bytes))?, input_hash]).map_err(|source| StreamRunError::Write { source })?;
        for (index, value) in values.iter().enumerate() {
            transaction
                .execute(
                    "INSERT INTO stream_values(value_index, name, value) VALUES (?1, ?2, ?3)",
                    params![
                        i64::try_from(index).map_err(|_| StreamRunError::Invalid)?,
                        value.name,
                        as_i64(u128::from(value.value))?
                    ],
                )
                .map_err(|source| StreamRunError::Write { source })?;
        }
        transaction
            .execute("DELETE FROM stream_outputs", [])
            .map_err(|source| StreamRunError::Write { source })?;
        for (index, output) in outputs.iter().enumerate() {
            transaction.execute("INSERT INTO stream_outputs(output_index, filename, content_type, bytes, hash, backend, reference, exported_path) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)", params![i64::try_from(index).map_err(|_| StreamRunError::Invalid)?, output.filename, output.content_type, as_i64(u128::from(output.bytes))?, output.hash, output.backend, output.reference, output.exported_path]).map_err(|source| StreamRunError::Write { source })?;
        }
        transaction
            .commit()
            .map_err(|source| StreamRunError::Write { source })?;
        Ok(())
    }

    pub fn fail(&mut self, message: &str) -> Result<(), StreamRunError> {
        let message: String = message.chars().take(MAX_ERROR_CHARS).collect();
        self.connection
            .execute(
                "UPDATE stream_run SET status='failed', error=?1 WHERE id=1",
                [message],
            )
            .map_err(|source| StreamRunError::Write { source })?;
        Ok(())
    }

    pub fn mark_output_exported(&mut self, index: usize, path: &str) -> Result<(), StreamRunError> {
        self.connection
            .execute(
                "UPDATE stream_outputs SET exported_path=?1 WHERE output_index=?2",
                params![
                    path,
                    i64::try_from(index).map_err(|_| StreamRunError::Invalid)?
                ],
            )
            .map_err(|source| StreamRunError::Write { source })?;
        Ok(())
    }
}

pub fn inspect_stream_run(path: &Path) -> Result<Option<StreamRunInspection>, StreamRunError> {
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|source| StreamRunError::Open { source })?;
    let exists = connection
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name='stream_run'",
            [],
            |_| Ok(()),
        )
        .optional()
        .map_err(|source| StreamRunError::Read { source })?
        .is_some();
    if !exists {
        return Ok(None);
    }
    let extended = has_column(&connection, "input_source")?;
    let sql = if extended {
        "SELECT workflow,input,status,error,duration_us,high,low,high_label,low_label,source_bytes,consumed_bytes,largest_batch_bytes,materialized_bytes,input_source,input_hash,input_accepts FROM stream_run WHERE id=1"
    } else {
        "SELECT workflow,input,status,error,duration_us,high,low,high_label,low_label,source_bytes,consumed_bytes,largest_batch_bytes,materialized_bytes,NULL,NULL,NULL FROM stream_run WHERE id=1"
    };
    let row = connection
        .query_row(sql, [], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<i64>>(4)?,
                row.get::<_, Option<i64>>(5)?,
                row.get::<_, Option<i64>>(6)?,
                row.get::<_, Option<String>>(7)?,
                row.get::<_, Option<String>>(8)?,
                row.get::<_, Option<i64>>(9)?,
                row.get::<_, Option<i64>>(10)?,
                row.get::<_, Option<i64>>(11)?,
                row.get::<_, Option<i64>>(12)?,
                row.get::<_, Option<String>>(13)?,
                row.get::<_, Option<String>>(14)?,
                row.get::<_, Option<String>>(15)?,
            ))
        })
        .map_err(|source| StreamRunError::Read { source })?;
    let mut statement = connection
        .prepare("SELECT name FROM stream_steps ORDER BY step_index")
        .map_err(|source| StreamRunError::Read { source })?;
    let steps = statement
        .query_map([], |row| row.get(0))
        .map_err(|source| StreamRunError::Read { source })?
        .collect::<Result<Vec<String>, _>>()
        .map_err(|source| StreamRunError::Read { source })?;
    let status = match row.2.as_str() {
        "running" => StreamRunStatus::Running,
        "completed" => StreamRunStatus::Completed,
        "failed" => StreamRunStatus::Failed(row.3.unwrap_or_else(|| "unknown failure".to_owned())),
        _ => return Err(StreamRunError::Invalid),
    };
    let values = if has_table(&connection, "stream_values")? {
        let mut statement = connection
            .prepare("SELECT name, value FROM stream_values ORDER BY value_index")
            .map_err(|source| StreamRunError::Read { source })?;
        statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })
            .map_err(|source| StreamRunError::Read { source })?
            .map(|row| {
                let (name, value) = row.map_err(|source| StreamRunError::Read { source })?;
                Ok(StreamValue {
                    name,
                    value: to_u64(value)?,
                })
            })
            .collect::<Result<Vec<_>, StreamRunError>>()?
    } else {
        Vec::new()
    };
    let outputs = if has_table(&connection, "stream_outputs")? {
        let exported = has_column(&connection, "exported_path")?;
        let sql = if exported {
            "SELECT filename, content_type, bytes, hash, backend, reference, exported_path FROM stream_outputs ORDER BY output_index"
        } else {
            "SELECT filename, content_type, bytes, hash, backend, reference, NULL FROM stream_outputs ORDER BY output_index"
        };
        let mut statement = connection
            .prepare(sql)
            .map_err(|source| StreamRunError::Read { source })?;
        statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, Option<String>>(6)?,
                ))
            })
            .map_err(|source| StreamRunError::Read { source })?
            .map(|row| {
                let (filename, content_type, bytes, hash, backend, reference, exported_path) =
                    row.map_err(|source| StreamRunError::Read { source })?;
                Ok(WorkflowOutputArtifact {
                    filename,
                    content_type,
                    bytes: to_u64(bytes)?,
                    hash,
                    backend,
                    reference,
                    exported_path,
                })
            })
            .collect::<Result<Vec<_>, StreamRunError>>()?
    } else {
        Vec::new()
    };
    let metrics = match (row.9, row.11, row.12) {
        (Some(source), Some(batch), Some(materialized)) => Some(StreamMetrics {
            source_bytes: to_u64(source)?,
            consumed_bytes: to_u64(row.10.ok_or(StreamRunError::Invalid)?)?,
            largest_batch_bytes: usize::try_from(batch).map_err(|_| StreamRunError::Invalid)?,
            materialized_bytes: to_u64(materialized)?,
        }),
        (None, None, None) => None,
        _ => return Err(StreamRunError::Invalid),
    };
    Ok(Some(StreamRunInspection {
        workflow: row.0,
        input: row.1,
        input_source: row.13,
        input_hash: row.14,
        input_accepts: row.15.map_or_else(Vec::new, |accepts| {
            accepts
                .split(',')
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
                .collect()
        }),
        status,
        steps,
        duration_us: row.4.map(to_u64).transpose()?,
        high: row.5.map(to_u64).transpose()?,
        low: row
            .6
            .map(|value| u32::try_from(value).map_err(|_| StreamRunError::Invalid))
            .transpose()?,
        high_label: row.7,
        low_label: row.8,
        metrics,
        values,
        outputs,
    }))
}

fn has_table(connection: &Connection, name: &str) -> Result<bool, StreamRunError> {
    connection
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1",
            [name],
            |_| Ok(()),
        )
        .optional()
        .map(|value| value.is_some())
        .map_err(|source| StreamRunError::Read { source })
}

fn has_column(connection: &Connection, name: &str) -> Result<bool, StreamRunError> {
    let mut statement = connection
        .prepare("SELECT name FROM pragma_table_info('stream_run') WHERE name=?1")
        .map_err(|source| StreamRunError::Read { source })?;
    statement
        .exists([name])
        .map_err(|source| StreamRunError::Read { source })
}

fn as_i64(value: u128) -> Result<i64, StreamRunError> {
    i64::try_from(value).map_err(|_| StreamRunError::Invalid)
}
fn to_u64(value: i64) -> Result<u64, StreamRunError> {
    u64::try_from(value).map_err(|_| StreamRunError::Invalid)
}
