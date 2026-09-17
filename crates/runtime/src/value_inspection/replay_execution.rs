use crate::{
    inspection::corrupt, journal::JournalError, journal_event::JournalEvent, payload::EventPayload,
};

use super::replay::{Builder, Component, expect_value, resolve_reuse};

pub(super) fn apply_execution_event(
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
            let planned = builder.durability_plan.get(&index);
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
                    durability_reason: planned.map(|planned| planned.reason.clone()),
                    planner_profile_id: planned.map(|planned| planned.profile_id.clone()),
                    planner_recompute_us: planned.and_then(|planned| planned.recompute_us),
                    planner_checkpoint_us: planned.and_then(|planned| planned.checkpoint_us),
                    planner_checkpoint_bytes: planned.and_then(|planned| planned.checkpoint_bytes),
                    planner_samples: planned.and_then(|planned| planned.samples),
                    planner_recorded_at_ms: planned.and_then(|planned| planned.recorded_at_ms),
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
