use std::collections::HashMap;

use crate::{
    inspection::corrupt, journal::JournalError, journal_event::JournalEvent, payload::EventPayload,
};

use super::{ValueComponentInspection, ValueRunInspection, ValueRunStatus, preview};

pub(super) fn expect_value(
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

pub(super) fn resolve_reuse(payload: EventPayload, current: &EventPayload) -> EventPayload {
    match payload {
        EventPayload::Reuse => current.clone(),
        payload => payload,
    }
}

pub(super) struct Component {
    pub(super) index: usize,
    pub(super) name: String,
    pub(super) hash: String,
    pub(super) input: EventPayload,
    pub(super) output: Option<EventPayload>,
    pub(super) duration_us: Option<u64>,
    pub(super) durable_after: Option<bool>,
    pub(super) checkpoint: Option<String>,
    pub(super) checkpoint_backend: Option<String>,
    pub(super) checkpoint_bytes: Option<u64>,
    pub(super) checkpoint_duration_us: Option<u64>,
    pub(super) durability_reason: Option<String>,
    pub(super) planner_profile_id: Option<String>,
    pub(super) planner_recompute_us: Option<u64>,
    pub(super) planner_checkpoint_us: Option<u64>,
    pub(super) planner_checkpoint_bytes: Option<u64>,
    pub(super) planner_samples: Option<u32>,
    pub(super) planner_recorded_at_ms: Option<u64>,
    pub(super) attempts: usize,
}

pub(super) struct PlannedDurability {
    pub(super) reason: String,
    pub(super) profile_id: String,
    pub(super) recompute_us: Option<u64>,
    pub(super) checkpoint_us: Option<u64>,
    pub(super) checkpoint_bytes: Option<u64>,
    pub(super) samples: Option<u32>,
    pub(super) recorded_at_ms: Option<u64>,
}

pub(super) struct Builder {
    pub(super) name: Option<String>,
    pub(super) input: EventPayload,
    pub(super) current_input: EventPayload,
    pub(super) start_index: usize,
    pub(super) components: Vec<Component>,
    pub(super) completed: Option<EventPayload>,
    pub(super) component_count: Option<usize>,
    pub(super) recovery_duration_us: Option<u64>,
    pub(super) durability_plan: HashMap<usize, PlannedDurability>,
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
                    planner_profile_id: component.planner_profile_id.clone(),
                    planner_recompute_us: component.planner_recompute_us,
                    planner_checkpoint_us: component.planner_checkpoint_us,
                    planner_checkpoint_bytes: component.planner_checkpoint_bytes,
                    planner_samples: component.planner_samples,
                    planner_recorded_at_ms: component.planner_recorded_at_ms,
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
            recompute_us,
            checkpoint_us,
            checkpoint_bytes,
            samples,
            recorded_at_ms,
            required: _,
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
                    recorded_at_ms,
                },
            );
        }
        event => super::replay_execution::apply_execution_event(
            sequence,
            event,
            started(sequence, builder)?,
        )?,
    }
    Ok(())
}

pub(super) fn started(
    sequence: i64,
    builder: &mut Option<Builder>,
) -> Result<&mut Builder, JournalError> {
    builder
        .as_mut()
        .ok_or_else(|| corrupt(sequence, "event appears before workflow start"))
}
