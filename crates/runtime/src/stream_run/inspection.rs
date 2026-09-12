use std::path::Path;

use rusqlite::{Connection, OpenFlags, OptionalExtension};

use super::{
    StreamEdgeMetrics, StreamMetrics, StreamRunError, StreamRunInspection, StreamRunStatus,
    StreamValue, WorkflowOutputArtifact, to_u64,
};

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
    let sql = if has_column(&connection, "input_source")? {
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
    let values = read_values(&connection)?;
    let outputs = read_outputs(&connection)?;
    let edges = read_edge_metrics(&connection)?;
    let metrics = match (row.9, row.11, row.12) {
        (Some(source), Some(batch), Some(materialized)) => Some(StreamMetrics {
            source_bytes: to_u64(source)?,
            consumed_bytes: to_u64(row.10.ok_or(StreamRunError::Invalid)?)?,
            largest_batch_bytes: usize::try_from(batch).map_err(|_| StreamRunError::Invalid)?,
            materialized_bytes: to_u64(materialized)?,
            edges,
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

fn read_edge_metrics(connection: &Connection) -> Result<Vec<StreamEdgeMetrics>, StreamRunError> {
    if !has_table(connection, "stream_edge_metrics")? {
        return Ok(Vec::new());
    }
    let mut statement = connection
        .prepare("SELECT name, bytes, peak_buffered_bytes, materialized, materialized_bytes FROM stream_edge_metrics ORDER BY edge_index")
        .map_err(|source| StreamRunError::Read { source })?;
    statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<i64>>(1)?,
                row.get::<_, Option<i64>>(2)?,
                row.get::<_, Option<i64>>(3)?,
                row.get::<_, Option<i64>>(4)?,
            ))
        })
        .map_err(|source| StreamRunError::Read { source })?
        .map(|row| {
            let (name, bytes, peak_buffered_bytes, materialized, materialized_bytes) =
                row.map_err(|source| StreamRunError::Read { source })?;
            let materialized = match materialized {
                Some(0) => Some(false),
                Some(1) => Some(true),
                Some(_) => return Err(StreamRunError::Invalid),
                None => None,
            };
            Ok(StreamEdgeMetrics {
                name,
                bytes: bytes.map(to_u64).transpose()?,
                peak_buffered_bytes: peak_buffered_bytes.map(to_u64).transpose()?,
                materialized,
                materialized_bytes: materialized_bytes.map(to_u64).transpose()?,
            })
        })
        .collect()
}

fn read_values(connection: &Connection) -> Result<Vec<StreamValue>, StreamRunError> {
    if !has_table(connection, "stream_values")? {
        return Ok(Vec::new());
    }
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
        .collect()
}

fn read_outputs(connection: &Connection) -> Result<Vec<WorkflowOutputArtifact>, StreamRunError> {
    if !has_table(connection, "stream_outputs")? {
        return Ok(Vec::new());
    }
    let sql = if has_column(connection, "exported_path")? {
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
        .collect()
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
