use crate::{identity::StepIdentity, journal::JournalError, journal_event::JournalEvent};

use super::{CellState, corrupt};

pub(super) fn apply_event(
    sequence: i64,
    event: JournalEvent,
    workflow_name: &str,
    fingerprint: &str,
    workflow_input: u32,
    steps: &[StepIdentity],
    state: &mut Option<CellState>,
) -> Result<(), JournalError> {
    match event {
        JournalEvent::WorkflowStarted {
            name,
            fingerprint: stored,
            input,
            component_count,
        } => {
            if state.is_some() {
                return Err(corrupt(sequence, "duplicate workflow start"));
            }
            if name.as_deref().is_some_and(|name| name != workflow_name)
                || stored != fingerprint
                || input != workflow_input
                || component_count.is_some_and(|count| count != steps.len())
            {
                return Err(JournalError::WorkflowChanged);
            }
            *state = Some(CellState::Ready {
                index: 0,
                input,
                checkpoint: None,
                retry: None,
            });
        }
        JournalEvent::ComponentStarted {
            index,
            name,
            hash,
            input,
            durable_after,
        } => {
            let step = steps
                .get(index)
                .ok_or_else(|| corrupt(sequence, "component index is out of bounds"))?;
            if step.name != name
                || step.hash.to_string() != hash
                || durable_after.is_some_and(|durable| durable != step.durable_after)
            {
                return Err(corrupt(sequence, "component identity does not match"));
            }
            match state {
                Some(CellState::Ready {
                    index: expected,
                    input: expected_input,
                    checkpoint: _,
                    retry: _,
                }) if *expected == index && *expected_input == input => {}
                Some(CellState::Running {
                    index: expected,
                    input: expected_input,
                    checkpoint: _,
                    ..
                }) if *expected == index && *expected_input == input => {}
                _ => return Err(corrupt(sequence, "unexpected component start")),
            }
            let checkpoint = match state {
                Some(CellState::Ready { checkpoint, .. })
                | Some(CellState::Running { checkpoint, .. }) => checkpoint.clone(),
                _ => None,
            };
            *state = Some(CellState::Running {
                index,
                input,
                checkpoint,
            });
        }
        JournalEvent::ComponentCompleted {
            index,
            output,
            duration_us: _,
        } => match state {
            Some(CellState::Running {
                index: expected,
                input,
                ..
            }) if *expected == index => {
                let next = index
                    .checked_add(1)
                    .ok_or_else(|| corrupt(sequence, "component index overflow"))?;
                *state = Some(CellState::Ready {
                    index: next,
                    input: output,
                    checkpoint: None,
                    retry: steps
                        .get(index)
                        .filter(|step| step.durable_after)
                        .map(|_| (index, *input)),
                });
            }
            _ => return Err(corrupt(sequence, "unexpected component completion")),
        },
        JournalEvent::CheckpointCreated {
            index,
            hash,
            backend,
            ..
        } => match state {
            Some(CellState::Ready {
                index: next,
                checkpoint: None,
                retry: Some((completed, _)),
                ..
            }) if *completed == index
                && *next
                    == index
                        .checked_add(1)
                        .ok_or_else(|| corrupt(sequence, "component index overflow"))? =>
            {
                let step = steps
                    .get(index)
                    .ok_or_else(|| corrupt(sequence, "checkpoint index is out of bounds"))?;
                if !step.durable_after {
                    return Err(corrupt(
                        sequence,
                        "checkpoint is not required by this workflow",
                    ));
                }
                if let Some(CellState::Ready {
                    checkpoint, retry, ..
                }) = state
                {
                    *checkpoint = Some(super::Checkpoint { hash, backend });
                    *retry = None;
                }
            }
            _ => return Err(corrupt(sequence, "unexpected checkpoint")),
        },
        JournalEvent::WorkflowCompleted { output } => match state {
            Some(CellState::Ready {
                index,
                input,
                checkpoint: None,
                retry: None,
            }) if *index == steps.len() && *input == output => {
                *state = Some(CellState::Completed { output });
            }
            _ => return Err(corrupt(sequence, "unexpected workflow completion")),
        },
        // diagnostic markers only -- carry no Cell state.
        JournalEvent::RecoveryTimed { .. } | JournalEvent::DurabilityPlanned { .. } => {}
    }
    Ok(())
}
