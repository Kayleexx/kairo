use std::collections::HashMap;

use crate::{
    inspection::corrupt, journal::JournalError, journal_event::JournalEvent, payload::EventPayload,
};

use super::{ValueComponentInspection, ValueRunInspection, ValueRunStatus, preview};

fn expect_value(
    sequence: i64,
    field: &str,
    payload: EventPayload,
) -> Result<EventPayload, JournalError> {
    match payload {
        EventPayload::Reuse => Err(corrupt(
            sequence,
            format!("{field} is an unresolved reuse payload"),
        )),
        payload => Ok(payload),
    }
}

fn resolve_reuse(payload: EventPayload, current: &EventPayload) -> EventPayload {
    match payload {
        EventPayload::Reuse => current.clone(),
        payload => payload,
    }
}

struct Component {
    index: usize,
    name: String,
    hash: String,
    input: EventPayload,
    output: Option<EventPayload>,
    duration_us: Option<u64>,
    durable_after: Option<bool>,
    checkpoint: Option<String>,
    checkpoint_backend: Option<String>,
    checkpoint_bytes: Option<u64>,
    checkpoint_duration_us: Option<u64>,
    durability_reason: Option<String>,
    attempts: usize,
}

pub(super) struct Builder {
    name: Option<String>,
    input: EventPayload,
    current_input: EventPayload,
    start_index: usize,
    components: Vec<Component>,
    completed: Option<EventPayload>,
    component_count: Option<usize>,
    recovery_duration_us: Option<u64>,
    durability_plan: HashMap<usize, String>,
}

impl Builder {
    pub(super) fn finish(self) -> ValueRunInspection {
        let next_index = self.start_index.saturating_add(self.components.len());
        let status = if let Some(output) = &self.completed {
            ValueRunStatus::Completed {
                output_preview: preview(output),
            }
        } else if let Some(component) = self.components.last() {
            if component.output.is_none() {
                ValueRunStatus::Interrupted {
                    step: component.name.clone(),
                }
            } else if component.durable_after == Some(true) && component.checkpoint.is_none() {
                ValueRunStatus::CheckpointPending {
                    step: component.name.clone(),
                }
            } else if self.component_count == Some(next_index) {
                ValueRunStatus::Finalizing
            } else {
                ValueRunStatus::Ready { next_index }
            }
        } else {
            ValueRunStatus::Ready {
                next_index: self.start_index,
            }
        };
        let metadata_complete = self.component_count.is_some()
            && self.components.iter().all(|component| {
                component.duration_us.is_some()
                    && component.durable_after.is_some()
                    && (component.checkpoint.is_none() || component.checkpoint_backend.is_some())
            });
        ValueRunInspection {
            name: self.name,
            input_preview: preview(&self.input),
            status,
            metadata_complete,
            components: self
                .components
                .iter()
                .map(|component| ValueComponentInspection {
                    index: component.index,
                    name: component.name.clone(),
                    hash: component.hash.clone(),
                    input_preview: preview(&component.input),
                    output_preview: component.output.as_ref().map(preview),
                    duration_us: component.duration_us,
                    durable_after: component.durable_after,
                    checkpoint: component.checkpoint.clone(),
                    checkpoint_backend: component.checkpoint_backend.clone(),
                    checkpoint_bytes: component.checkpoint_bytes,
                    checkpoint_duration_us: component.checkpoint_duration_us,
                    durability_reason: component.durability_reason.clone(),
                    attempts: component.attempts,
                })
                .collect(),
            recovery_duration_us: self.recovery_duration_us,
        }
    }
}

pub(super) fn apply_event(
    sequence: i64,
    event: JournalEvent,
    builder: &mut Option<Builder>,
) -> Result<(), JournalError> {
    match event {
        JournalEvent::WorkflowStarted {
            name,
            input,
            component_count,
            start_index,
            start_input,
            ..
        } => {
            if builder.is_some() {
                return Err(corrupt(sequence, "duplicate workflow start"));
            }
            let input = expect_value(sequence, "input", input)?;
            let start_input = start_input
                .map(|payload| expect_value(sequence, "start input", payload))
                .transpose()?
                .unwrap_or_else(|| input.clone());
            *builder = Some(Builder {
                name,
                input: start_input.clone(),
                current_input: start_input,
                start_index: start_index.unwrap_or(0),
                components: Vec::new(),
                completed: None,
                component_count,
                recovery_duration_us: None,
                durability_plan: HashMap::new(),
            });
        }
        JournalEvent::RecoveryTimed { duration_us } => {
            started(sequence, builder)?.recovery_duration_us = Some(duration_us);
        }
        JournalEvent::DurabilityPlanned {
            index,
            profile_id,
            reason,
            ..
        } => {
            started(sequence, builder)?
                .durability_plan
                .insert(index, format!("{profile_id} · {reason}"));
        }
        event => apply_execution_event(sequence, event, started(sequence, builder)?)?,
    }
    Ok(())
}

fn apply_execution_event(
    sequence: i64,
    event: JournalEvent,
    builder: &mut Builder,
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
        } => {
            let input = resolve_reuse(input, &builder.current_input);
            let durability_reason = builder.durability_plan.get(&index).cloned();
            start_component(
                sequence,
                builder,
                Component {
                    index,
                    name,
                    hash,
                    input,
                    output: None,
                    duration_us: None,
                    durable_after,
                    checkpoint: None,
                    checkpoint_backend: None,
                    checkpoint_bytes: None,
                    checkpoint_duration_us: None,
                    durability_reason,
                    attempts: 1,
                },
            )
        }
        JournalEvent::ComponentCompleted {
            index,
            output,
            duration_us,
        } => {
            let output = expect_value(sequence, "output", output)?;
            complete_component(sequence, builder, index, output, duration_us)
        }
        JournalEvent::CheckpointCreated {
            index,
            hash,
            backend,
            bytes,
            duration_us,
        } => record_checkpoint(sequence, builder, index, hash, backend, bytes, duration_us),
        JournalEvent::WorkflowCompleted { output } => {
            let output = resolve_reuse(output, &builder.current_input);
            complete_workflow(sequence, builder, output)
        }
        JournalEvent::WorkflowStarted { .. } => Err(corrupt(sequence, "duplicate workflow start")),
        JournalEvent::RecoveryTimed { .. } => Err(corrupt(sequence, "unexpected recovery marker")),
        JournalEvent::DurabilityPlanned { .. } => {
            Err(corrupt(sequence, "unexpected durability plan marker"))
        }
    }
}

fn start_component(
    sequence: i64,
    builder: &mut Builder,
    component: Component,
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
        .and_then(|step| step.output.clone())
        .unwrap_or_else(|| builder.input.clone());
    let expected_index = builder.start_index.saturating_add(builder.components.len());
    if component.index != expected_index || component.input != expected_input {
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
    builder: &mut Builder,
    index: usize,
    output: EventPayload,
    duration_us: Option<u64>,
) -> Result<(), JournalError> {
    let component = builder
        .components
        .last_mut()
        .filter(|component| component.index == index && component.output.is_none())
        .ok_or_else(|| corrupt(sequence, "unexpected component completion"))?;
    component.output = Some(output.clone());
    component.duration_us = duration_us;
    builder.current_input = output;
    Ok(())
}

fn record_checkpoint(
    sequence: i64,
    builder: &mut Builder,
    index: usize,
    hash: String,
    backend: Option<String>,
    bytes: Option<u64>,
    duration_us: Option<u64>,
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
    component.checkpoint_bytes = bytes;
    component.checkpoint_duration_us = duration_us;
    Ok(())
}

fn complete_workflow(
    sequence: i64,
    builder: &mut Builder,
    output: EventPayload,
) -> Result<(), JournalError> {
    let component = builder
        .components
        .last()
        .filter(|component| component.output.as_ref() == Some(&output))
        .ok_or_else(|| corrupt(sequence, "unexpected workflow completion"))?;
    if component.durable_after == Some(true) && component.checkpoint.is_none() {
        return Err(corrupt(
            sequence,
            "workflow completed before its checkpoint",
        ));
    }
    let seen = builder.start_index.saturating_add(builder.components.len());
    if builder.component_count.is_some_and(|count| count != seen) {
        return Err(corrupt(
            sequence,
            "workflow completed before all components",
        ));
    }
    builder.completed = Some(output);
    Ok(())
}

fn started(sequence: i64, builder: &mut Option<Builder>) -> Result<&mut Builder, JournalError> {
    builder
        .as_mut()
        .ok_or_else(|| corrupt(sequence, "event appears before workflow start"))
}
