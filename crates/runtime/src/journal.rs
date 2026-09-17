use crate::journal_event::{JournalEvent, decode_row};
use crate::journal_schema;
pub(crate) use crate::journal_schema::SCHEMA_VERSION;
use crate::payload::{self, EncodedPayload};
use rusqlite::{Connection, ErrorCode, params};
use std::{
    fs::{self, File},
    path::{Path, PathBuf},
    time::Duration,
};
use thiserror::Error;

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
    Busy,
    #[error("failed to lock journal `{path}`")]
    Lock {
        path: PathBuf,
        #[source]
        source: std::io::Error,
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
    #[error("journal belongs to another workflow; choose a new state path")]
    WorkflowChanged,
    #[error("invalid Cell state: {message}")]
    InvalidState { message: String },
}

pub(crate) struct Journal {
    connection: Connection,
    _lock: File,
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

        let lock_path = path.with_extension("lock");
        let lock = File::create(&lock_path).map_err(|source| JournalError::Lock {
            path: lock_path.clone(),
            source,
        })?;
        lock.try_lock().map_err(|source| match source {
            std::fs::TryLockError::WouldBlock => JournalError::Busy,
            std::fs::TryLockError::Error(source) => JournalError::Lock {
                path: lock_path,
                source,
            },
        })?;

        let connection = Connection::open(path).map_err(|source| JournalError::Open { source })?;
        connection
            .busy_timeout(LOCK_TIMEOUT)
            .map_err(|source| JournalError::Configure { source })?;
        connection
            .pragma_update(None, "journal_mode", "WAL")
            .map_err(classify_lock_error)?;
        connection
            .pragma_update(None, "synchronous", "FULL")
            .map_err(|source| JournalError::Configure { source })?;

        journal_schema::initialize(&connection)?;
        Ok(Self {
            connection,
            _lock: lock,
        })
    }

    pub(crate) fn append(&self, event: &JournalEvent) -> Result<(), JournalError> {
        let row = EventRow::from_event(event)?;
        self.connection
            .execute(
                "INSERT INTO events(\
                    kind, step_index, workflow_fingerprint, component_name, component_hash, \
                    input_value, output_value, artifact_hash, workflow_name, duration_us, \
                    durability_required, component_count, artifact_backend, artifact_bytes, \
                    planner_profile_id, planner_reason, planner_recompute_us, \
                    planner_checkpoint_us, planner_checkpoint_bytes, planner_samples, \
                    start_index, start_input, \
                    input_payload_kind, input_payload_inline, input_payload_hash, \
                    input_payload_bytes, output_payload_kind, output_payload_inline, \
                    output_payload_hash, output_payload_bytes, start_payload_kind, \
                    start_payload_inline, start_payload_hash, start_payload_bytes, \
                    planner_recorded_at_ms\
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, \
                    ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29, ?30, ?31, ?32, ?33, ?34, ?35)",
                params![
                    row.kind,
                    row.index,
                    row.fingerprint,
                    row.name,
                    row.hash,
                    row.input,
                    row.output,
                    row.artifact_hash,
                    row.workflow_name,
                    row.duration_us,
                    row.durable_after,
                    row.component_count,
                    row.artifact_backend,
                    row.artifact_bytes,
                    row.planner_profile_id,
                    row.planner_reason,
                    row.planner_recompute_us,
                    row.planner_checkpoint_us,
                    row.planner_checkpoint_bytes,
                    row.planner_samples,
                    row.start_index,
                    row.start_input,
                    row.input_payload.kind,
                    row.input_payload.inline,
                    row.input_payload.hash,
                    row.input_payload.bytes,
                    row.output_payload.kind,
                    row.output_payload.inline,
                    row.output_payload.hash,
                    row.output_payload.bytes,
                    row.start_payload.kind,
                    row.start_payload.inline,
                    row.start_payload.hash,
                    row.start_payload.bytes,
                    row.planner_recorded_at_ms,
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
                        component_hash, input_value, output_value, artifact_hash, workflow_name, \
                        duration_us, durability_required, component_count, artifact_backend, \
                        artifact_bytes, planner_profile_id, planner_reason, \
                        planner_recompute_us, planner_checkpoint_us, planner_checkpoint_bytes, \
                        planner_samples, start_index, \
                        start_input, input_payload_kind, input_payload_inline, \
                        input_payload_hash, input_payload_bytes, output_payload_kind, \
                        output_payload_inline, output_payload_hash, output_payload_bytes, \
                        start_payload_kind, start_payload_inline, start_payload_hash, \
                        start_payload_bytes, planner_recorded_at_ms \
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
}

#[derive(Default)]
struct EventRow<'a> {
    kind: &'static str,
    index: Option<i64>,
    fingerprint: Option<&'a str>,
    name: Option<&'a str>,
    hash: Option<&'a str>,
    input: Option<i64>,
    input_payload: EncodedPayload<'a>,
    output: Option<i64>,
    output_payload: EncodedPayload<'a>,
    artifact_hash: Option<&'a str>,
    workflow_name: Option<&'a str>,
    duration_us: Option<i64>,
    durable_after: Option<i64>,
    component_count: Option<i64>,
    artifact_backend: Option<&'a str>,
    artifact_bytes: Option<i64>,
    planner_profile_id: Option<&'a str>,
    planner_reason: Option<&'a str>,
    planner_recompute_us: Option<i64>,
    planner_checkpoint_us: Option<i64>,
    planner_checkpoint_bytes: Option<i64>,
    planner_samples: Option<i64>,
    planner_recorded_at_ms: Option<i64>,
    start_index: Option<i64>,
    start_input: Option<i64>,
    start_payload: EncodedPayload<'a>,
}

impl<'a> EventRow<'a> {
    fn from_event(event: &'a JournalEvent) -> Result<Self, JournalError> {
        Ok(match event {
            JournalEvent::WorkflowStarted {
                name,
                fingerprint,
                input,
                component_count,
                start_index,
                start_input,
            } => {
                let (input, input_payload) = payload::encode(input);
                let (start_input, start_payload) =
                    start_input.as_ref().map(payload::encode).unzip();
                Self {
                    kind: "workflow_started",
                    fingerprint: Some(fingerprint.as_str()),
                    input,
                    input_payload,
                    workflow_name: name.as_deref(),
                    component_count: component_count.map(index_value).transpose()?,
                    start_index: start_index.map(index_value).transpose()?,
                    start_input: start_input.flatten(),
                    start_payload: start_payload.unwrap_or_default(),
                    ..Self::default()
                }
            }
            JournalEvent::ComponentStarted {
                index,
                name,
                hash,
                input,
                durable_after,
            } => {
                let (input, input_payload) = payload::encode(input);
                Self {
                    kind: "component_started",
                    index: Some(index_value(*index)?),
                    name: Some(name.as_str()),
                    hash: Some(hash.as_str()),
                    input,
                    input_payload,
                    durable_after: durable_after.map(i64::from),
                    ..Self::default()
                }
            }
            JournalEvent::ComponentCompleted {
                index,
                output,
                duration_us,
            } => {
                let (output, output_payload) = payload::encode(output);
                Self {
                    kind: "component_completed",
                    index: Some(index_value(*index)?),
                    output,
                    output_payload,
                    duration_us: duration_us.map(duration_value),
                    ..Self::default()
                }
            }
            JournalEvent::CheckpointCreated {
                index,
                hash,
                backend,
                bytes,
                duration_us,
            } => Self {
                kind: "checkpoint_created",
                index: Some(index_value(*index)?),
                artifact_hash: Some(hash.as_str()),
                artifact_backend: backend.as_deref(),
                artifact_bytes: bytes.map(|bytes| bytes.min(i64::MAX as u64) as i64),
                duration_us: duration_us.map(duration_value),
                ..Self::default()
            },
            JournalEvent::WorkflowCompleted { output } => {
                let (output, output_payload) = payload::encode(output);
                Self {
                    kind: "workflow_completed",
                    output,
                    output_payload,
                    ..Self::default()
                }
            }
            JournalEvent::RecoveryTimed { duration_us } => Self {
                kind: "recovery_timed",
                duration_us: Some(duration_value(*duration_us)),
                ..Self::default()
            },
            JournalEvent::DurabilityPlanned {
                index,
                required,
                profile_id,
                reason,
                recompute_us,
                checkpoint_us,
                checkpoint_bytes,
                samples,
                recorded_at_ms,
            } => Self {
                kind: "durability_planned",
                index: Some(index_value(*index)?),
                durable_after: Some(i64::from(*required)),
                planner_profile_id: Some(profile_id.as_str()),
                planner_reason: Some(reason.as_str()),
                planner_recompute_us: recompute_us.map(duration_value),
                planner_checkpoint_us: checkpoint_us.map(duration_value),
                planner_checkpoint_bytes: checkpoint_bytes
                    .map(|bytes| bytes.min(i64::MAX as u64) as i64),
                planner_samples: samples.map(i64::from),
                planner_recorded_at_ms: recorded_at_ms.map(duration_value),
                ..Self::default()
            },
        })
    }
}

fn index_value(index: usize) -> Result<i64, JournalError> {
    i64::try_from(index).map_err(|_| JournalError::InvalidState {
        message: "component index is too large".to_owned(),
    })
}

fn duration_value(duration_us: u64) -> i64 {
    duration_us.min(i64::MAX as u64) as i64
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
        let _ = source;
        JournalError::Busy
    } else {
        JournalError::Configure { source }
    }
}
