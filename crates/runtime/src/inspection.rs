use std::{path::Path, time::Duration};

use rusqlite::{Connection, ErrorCode, OpenFlags};

use crate::{
    journal::{JournalError, SCHEMA_VERSION},
    journal_event::{JournalEvent, decode_row},
};

pub(crate) const READ_TIMEOUT: Duration = Duration::from_millis(250);

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CellStatus {
    Ready { next_index: usize },
    Interrupted { step: String },
    CheckpointPending { step: String },
    Finalizing,
    Completed { output: u32 },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ComponentInspection {
    pub index: usize,
    pub name: String,
    pub hash: String,
    pub input: u32,
    pub output: Option<u32>,
    pub duration_us: Option<u64>,
    pub durable_after: Option<bool>,
    pub checkpoint: Option<String>,
    pub checkpoint_backend: Option<String>,
    pub attempts: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CellInspection {
    pub name: Option<String>,
    pub input: u32,
    pub status: CellStatus,
    pub components: Vec<ComponentInspection>,
    pub metadata_complete: bool,
}

pub fn inspect_cell(path: impl AsRef<Path>) -> Result<CellInspection, JournalError> {
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|source| JournalError::Open { source })?;
    connection
        .busy_timeout(READ_TIMEOUT)
        .map_err(|source| JournalError::Configure { source })?;
    let version = connection
        .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
        .map_err(classify_read_error)?;
    let query = event_query(version)?;
    let mut statement = connection.prepare(query).map_err(classify_read_error)?;
    let mut rows = statement.query([]).map_err(classify_read_error)?;
    let mut builder = None;
    let mut expected_sequence = 1_i64;

    while let Some(row) = rows.next().map_err(classify_read_error)? {
        let (sequence, event) = decode_row(row)?;
        if sequence != expected_sequence {
            return Err(corrupt(sequence, "event sequence is not contiguous"));
        }
        expected_sequence = expected_sequence
            .checked_add(1)
            .ok_or_else(|| corrupt(sequence, "event sequence overflow"))?;
        apply_event(sequence, event, &mut builder)?;
    }

    builder
        .ok_or_else(|| corrupt(0, "journal has no workflow start"))?
        .finish()
}

struct InspectionBuilder {
    name: Option<String>,
    input: u32,
    components: Vec<ComponentInspection>,
    completed: Option<u32>,
    component_count: Option<usize>,
}

impl InspectionBuilder {
    fn finish(self) -> Result<CellInspection, JournalError> {
        let status = if let Some(output) = self.completed {
            CellStatus::Completed { output }
        } else if let Some(component) = self.components.last() {
            if component.output.is_none() {
                CellStatus::Interrupted {
                    step: component.name.clone(),
                }
            } else if component.durable_after == Some(true) && component.checkpoint.is_none() {
                CellStatus::CheckpointPending {
                    step: component.name.clone(),
                }
            } else if self.component_count == Some(self.components.len()) {
                CellStatus::Finalizing
            } else {
                CellStatus::Ready {
                    next_index: self.components.len(),
                }
            }
        } else {
            CellStatus::Ready { next_index: 0 }
        };
        Ok(CellInspection {
            name: self.name,
            input: self.input,
            status,
            metadata_complete: self.component_count.is_some()
                && self.components.iter().all(|component| {
                    component.duration_us.is_some()
                        && component.durable_after.is_some()
                        && (component.checkpoint.is_none()
                            || component.checkpoint_backend.is_some())
                }),
            components: self.components,
        })
    }
}

pub(crate) fn event_query(version: i64) -> Result<&'static str, JournalError> {
    match version {
        1 => Ok(
            "SELECT sequence, kind, step_index, workflow_fingerprint, component_name, \
                 component_hash, input_value, output_value, NULL, NULL, NULL, NULL, NULL, NULL \
                 FROM events ORDER BY sequence",
        ),
        2 => Ok(
            "SELECT sequence, kind, step_index, workflow_fingerprint, component_name, \
                 component_hash, input_value, output_value, artifact_hash, NULL, NULL, NULL, NULL, NULL \
                 FROM events ORDER BY sequence",
        ),
        3 => Ok(
            "SELECT sequence, kind, step_index, workflow_fingerprint, component_name, \
             component_hash, input_value, output_value, artifact_hash, workflow_name, \
             duration_us, durability_required, component_count, NULL FROM events ORDER BY sequence",
        ),
        version if version == SCHEMA_VERSION => Ok(
            "SELECT sequence, kind, step_index, workflow_fingerprint, component_name, \
             component_hash, input_value, output_value, artifact_hash, workflow_name, \
             duration_us, durability_required, component_count, artifact_backend \
             FROM events ORDER BY sequence",
        ),
        found => Err(JournalError::UnsupportedSchema { found }),
    }
}

fn apply_event(
    sequence: i64,
    event: JournalEvent,
    builder: &mut Option<InspectionBuilder>,
) -> Result<(), JournalError> {
    match event {
        JournalEvent::WorkflowStarted {
            name,
            input,
            component_count,
            ..
        } => {
            if builder.is_some() {
                return Err(corrupt(sequence, "duplicate workflow start"));
            }
            *builder = Some(InspectionBuilder {
                name,
                input,
                components: Vec::new(),
                completed: None,
                component_count,
            });
        }
        event => apply_execution_event(sequence, event, started(sequence, builder)?)?,
    }
    Ok(())
}

fn apply_execution_event(
    sequence: i64,
    event: JournalEvent,
    builder: &mut InspectionBuilder,
) -> Result<(), JournalError> {
    if builder.completed.is_some() {
        return Err(corrupt(sequence, "event follows workflow completion"));
    }
    match event {
        JournalEvent::ComponentStarted {
            index,
            name,
            hash,
            input,
            durable_after,
        } => start_component(
            sequence,
            builder,
            ComponentInspection {
                index,
                name,
                hash,
                input,
                output: None,
                duration_us: None,
                durable_after,
                checkpoint: None,
                checkpoint_backend: None,
                attempts: 1,
            },
        ),
        JournalEvent::ComponentCompleted {
            index,
            output,
            duration_us,
        } => complete_component(sequence, builder, index, output, duration_us),
        JournalEvent::CheckpointCreated {
            index,
            hash,
            backend,
        } => record_checkpoint(sequence, builder, index, hash, backend),
        JournalEvent::WorkflowCompleted { output } => complete_workflow(sequence, builder, output),
        JournalEvent::WorkflowStarted { .. } => Err(corrupt(sequence, "duplicate workflow start")),
    }
}

fn start_component(
    sequence: i64,
    builder: &mut InspectionBuilder,
    component: ComponentInspection,
) -> Result<(), JournalError> {
    if let Some(current) = builder
        .components
        .last_mut()
        .filter(|step| step.output.is_none())
    {
        if current.index != component.index
            || current.name != component.name
            || current.hash != component.hash
            || current.input != component.input
            || matches!(
                (current.durable_after, component.durable_after),
                (Some(left), Some(right)) if left != right
            )
        {
            return Err(corrupt(
                sequence,
                "retry does not match the interrupted component",
            ));
        }
        current.durable_after = current.durable_after.or(component.durable_after);
        current.attempts = current
            .attempts
            .checked_add(1)
            .ok_or_else(|| corrupt(sequence, "component attempt count overflow"))?;
        return Ok(());
    }
    let expected_input = builder
        .components
        .last()
        .and_then(|step| step.output)
        .unwrap_or(builder.input);
    if component.index != builder.components.len() || component.input != expected_input {
        return Err(corrupt(sequence, "unexpected component start"));
    }
    if builder
        .component_count
        .is_some_and(|count| component.index >= count)
    {
        return Err(corrupt(
            sequence,
            "component index exceeds the recorded workflow",
        ));
    }
    builder.components.push(component);
    Ok(())
}

fn complete_component(
    sequence: i64,
    builder: &mut InspectionBuilder,
    index: usize,
    output: u32,
    duration_us: Option<u64>,
) -> Result<(), JournalError> {
    let component = builder
        .components
        .last_mut()
        .filter(|component| component.index == index && component.output.is_none())
        .ok_or_else(|| corrupt(sequence, "unexpected component completion"))?;
    component.output = Some(output);
    component.duration_us = duration_us;
    Ok(())
}

fn record_checkpoint(
    sequence: i64,
    builder: &mut InspectionBuilder,
    index: usize,
    hash: String,
    backend: Option<String>,
) -> Result<(), JournalError> {
    let component = builder
        .components
        .last_mut()
        .filter(|component| component.index == index && component.output.is_some())
        .ok_or_else(|| corrupt(sequence, "unexpected checkpoint"))?;
    if component.durable_after == Some(false) || component.checkpoint.is_some() {
        return Err(corrupt(sequence, "unexpected checkpoint"));
    }
    component.durable_after = Some(true);
    component.checkpoint = Some(hash);
    component.checkpoint_backend = backend;
    Ok(())
}

fn complete_workflow(
    sequence: i64,
    builder: &mut InspectionBuilder,
    output: u32,
) -> Result<(), JournalError> {
    let component = builder
        .components
        .last()
        .filter(|component| component.output == Some(output))
        .ok_or_else(|| corrupt(sequence, "unexpected workflow completion"))?;
    if component.durable_after == Some(true) && component.checkpoint.is_none() {
        return Err(corrupt(
            sequence,
            "workflow completed before its checkpoint",
        ));
    }
    if builder
        .component_count
        .is_some_and(|count| count != builder.components.len())
    {
        return Err(corrupt(
            sequence,
            "workflow completed before all components",
        ));
    }
    builder.completed = Some(output);
    Ok(())
}

fn started(
    sequence: i64,
    builder: &mut Option<InspectionBuilder>,
) -> Result<&mut InspectionBuilder, JournalError> {
    builder
        .as_mut()
        .ok_or_else(|| corrupt(sequence, "event appears before workflow start"))
}

pub(crate) fn classify_read_error(source: rusqlite::Error) -> JournalError {
    if matches!(
        source.sqlite_error_code(),
        Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked)
    ) {
        let _ = source;
        JournalError::Busy
    } else {
        JournalError::Read { source }
    }
}

fn corrupt(sequence: i64, message: impl Into<String>) -> JournalError {
    JournalError::Corrupt {
        sequence,
        message: message.into(),
    }
}
