use crate::journal::JournalError;
use crate::journal_event::JournalEvent;
use crate::payload::{EventPayload, expect_scalar};

use super::{CellInspection, ComponentInspection, corrupt};

pub(super) struct InspectionBuilder {
    name: Option<String>,
    input: u32,
    // the value a `Reuse` payload resolves to: the workflow's input until the first component
    // completes, then that component's output, and so on -- unlike `components.last().output`,
    // this stays correct across a retry (a second `ComponentStarted` for the same index, whose
    // own output is still `None`).
    current_input: u32,
    start_index: usize,
    components: Vec<ComponentInspection>,
    completed: Option<u32>,
    component_count: Option<usize>,
    recovery_duration_us: Option<u64>,
    // keyed by step index; merged into each ComponentInspection as it's created.
    durability_plan: std::collections::HashMap<usize, PlannedDurability>,
}

struct PlannedDurability {
    reason: String,
    profile_id: String,
    recompute_us: Option<u64>,
    checkpoint_us: Option<u64>,
    checkpoint_bytes: Option<u64>,
    samples: Option<u32>,
}

impl InspectionBuilder {
    pub(super) fn finish(self) -> Result<CellInspection, JournalError> {
        let next_index = self.start_index.saturating_add(self.components.len());
        let status = if let Some(output) = self.completed {
            super::CellStatus::Completed { output }
        } else if let Some(component) = self.components.last() {
            if component.output.is_none() {
                super::CellStatus::Interrupted {
                    step: component.name.clone(),
                }
            } else if component.durable_after == Some(true) && component.checkpoint.is_none() {
                super::CellStatus::CheckpointPending {
                    step: component.name.clone(),
                }
            } else if self.component_count == Some(next_index) {
                super::CellStatus::Finalizing
            } else {
                super::CellStatus::Ready { next_index }
            }
        } else {
            super::CellStatus::Ready {
                next_index: self.start_index,
            }
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
            recovery_duration_us: self.recovery_duration_us,
        })
    }
}

pub(super) fn apply_event(
    sequence: i64,
    event: JournalEvent,
    builder: &mut Option<InspectionBuilder>,
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
            let input = expect_scalar(sequence, "input", input)?;
            let start_input = start_input
                .map(|payload| expect_scalar(sequence, "start input", payload))
                .transpose()?;
            let input = start_input.unwrap_or(input);
            *builder = Some(InspectionBuilder {
                name,
                // a group's journal seeds its first component from its own start input, not the
                // whole run's original fingerprint input -- fall back to `input` for pre-Phase-15
                // journals, which never diverge from it.
                input,
                current_input: input,
                start_index: start_index.unwrap_or(0),
                components: Vec::new(),
                completed: None,
                component_count,
                recovery_duration_us: None,
                durability_plan: std::collections::HashMap::new(),
            });
        }
        // a diagnostic marker, not an execution step -- allowed even after completion, since
        // reopening an already-completed journal is itself a real (if trivial) recovery.
        JournalEvent::RecoveryTimed { duration_us } => {
            started(sequence, builder)?.recovery_duration_us = Some(duration_us);
        }
        JournalEvent::DurabilityPlanned {
            index,
            profile_id,
            reason,
            recompute_us,
            checkpoint_us,
            checkpoint_bytes,
            samples,
            ..
        } => {
            started(sequence, builder)?.durability_plan.insert(
                index,
                PlannedDurability {
                    reason: format!("{profile_id} · {reason}"),
                    profile_id,
                    recompute_us,
                    checkpoint_us,
                    checkpoint_bytes,
                    samples,
                },
            );
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
        } => {
            let input = resolve_scalar_reuse(sequence, input, builder.current_input)?;
            let planned = builder.durability_plan.get(&index);
            start_component(
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
                    checkpoint_bytes: None,
                    checkpoint_duration_us: None,
                    durability_reason: planned.map(|planned| planned.reason.clone()),
                    planner_profile_id: planned.map(|planned| planned.profile_id.clone()),
                    planner_recompute_us: planned.and_then(|planned| planned.recompute_us),
                    planner_checkpoint_us: planned.and_then(|planned| planned.checkpoint_us),
                    planner_checkpoint_bytes: planned.and_then(|planned| planned.checkpoint_bytes),
                    planner_samples: planned.and_then(|planned| planned.samples),
                    attempts: 1,
                },
            )
        }
        JournalEvent::ComponentCompleted {
            index,
            output,
            duration_us,
        } => {
            let output = expect_scalar(sequence, "output", output)?;
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
            let output = resolve_scalar_reuse(sequence, output, builder.current_input)?;
            complete_workflow(sequence, builder, output)
        }
        JournalEvent::WorkflowStarted { .. } => Err(corrupt(sequence, "duplicate workflow start")),
        // handled directly by `apply_event`, never routed here.
        JournalEvent::RecoveryTimed { .. } => Err(corrupt(sequence, "unexpected recovery marker")),
        JournalEvent::DurabilityPlanned { .. } => {
            Err(corrupt(sequence, "unexpected durability plan marker"))
        }
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
    builder.current_input = output;
    Ok(())
}

fn record_checkpoint(
    sequence: i64,
    builder: &mut InspectionBuilder,
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

fn resolve_scalar_reuse(
    sequence: i64,
    payload: EventPayload,
    current: u32,
) -> Result<u32, JournalError> {
    match payload {
        EventPayload::Reuse => Ok(current),
        payload => expect_scalar(sequence, "payload", payload),
    }
}

fn started(
    sequence: i64,
    builder: &mut Option<InspectionBuilder>,
) -> Result<&mut InspectionBuilder, JournalError> {
    builder
        .as_mut()
        .ok_or_else(|| corrupt(sequence, "event appears before workflow start"))
}
