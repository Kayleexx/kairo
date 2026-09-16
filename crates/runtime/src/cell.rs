use std::time::Instant;

use crate::{
    durability_plan::AutoResolution,
    identity::{StepIdentity, workflow_fingerprint},
    journal::{Journal, JournalError},
    journal_event::JournalEvent,
    payload::EventPayload,
};

mod replay;

#[derive(Clone)]
struct Checkpoint {
    hash: String,
    backend: Option<String>,
}

enum CellState {
    Ready {
        index: usize,
        input: EventPayload,
        checkpoint: Option<Checkpoint>,
        retry: Option<(usize, EventPayload)>,
    },
    Running {
        index: usize,
        input: EventPayload,
        checkpoint: Option<Checkpoint>,
    },
    Completed {
        output: EventPayload,
    },
}

pub(crate) enum PendingStep {
    Local {
        index: usize,
        input: EventPayload,
    },
    Checkpoint {
        index: usize,
        input: EventPayload,
        hash: String,
        backend: Option<String>,
    },
}

pub(crate) struct Cell {
    journal: Journal,
    state: CellState,
    resumed: bool,
}

impl Cell {
    pub(crate) fn open(
        journal: Journal,
        workflow_name: &str,
        input: EventPayload,
        steps: &[StepIdentity],
        auto_plan: &[AutoResolution],
        start_index: usize,
        start_input: EventPayload,
    ) -> Result<Self, JournalError> {
        let fingerprint = workflow_fingerprint(workflow_name, &input, steps);
        let mut state = None;
        let replay_started = Instant::now();
        let found = journal.replay(|sequence, event| {
            replay::apply_event(
                sequence,
                event,
                workflow_name,
                &fingerprint,
                &input,
                steps,
                start_index,
                &start_input,
                &mut state,
            )
        })?;
        // no prior journal to recover from, so don't fake a ~0 duration.
        if found {
            let duration_us = replay_started
                .elapsed()
                .as_micros()
                .min(u128::from(u64::MAX)) as u64;
            journal.append(&JournalEvent::RecoveryTimed { duration_us })?;
        }

        if !found {
            journal.append(&JournalEvent::WorkflowStarted {
                name: Some(workflow_name.to_owned()),
                fingerprint,
                input: input.clone(),
                component_count: Some(steps.len()),
                start_index: Some(start_index),
                start_input: Some(start_input.clone()),
            })?;
            // journaled before any component runs, so a resumed cell always finds the plan.
            for resolution in auto_plan {
                journal.append(&JournalEvent::DurabilityPlanned {
                    index: resolution.index,
                    required: resolution.required,
                    profile_id: resolution.profile_id.clone(),
                    reason: resolution.reason.clone(),
                    recompute_us: resolution.profile.map(|profile| profile.recompute_us),
                    checkpoint_us: resolution.profile.map(|profile| profile.checkpoint_us),
                    checkpoint_bytes: resolution.profile.map(|profile| profile.checkpoint_bytes),
                    samples: resolution.profile.map(|profile| profile.samples),
                })?;
            }
            state = Some(CellState::Ready {
                index: start_index,
                input: start_input,
                checkpoint: None,
                retry: None,
            });
        }

        let state = match state.ok_or_else(|| corrupt(0, "journal has no workflow start"))? {
            CellState::Running {
                index,
                input,
                checkpoint,
            } => CellState::Ready {
                index,
                input,
                checkpoint,
                retry: None,
            },
            CellState::Ready {
                retry: Some((index, input)),
                ..
            } => CellState::Ready {
                index,
                input,
                checkpoint: None,
                retry: None,
            },
            state => state,
        };
        Ok(Self {
            journal,
            state,
            resumed: found,
        })
    }

    pub(crate) fn next(&self, step_count: usize) -> Option<PendingStep> {
        match &self.state {
            CellState::Ready {
                index,
                input,
                checkpoint: Some(checkpoint),
                ..
            } if self.resumed && *index < step_count => Some(PendingStep::Checkpoint {
                index: *index,
                input: input.clone(),
                hash: checkpoint.hash.clone(),
                backend: checkpoint.backend.clone(),
            }),
            CellState::Ready { index, input, .. } if *index < step_count => {
                Some(PendingStep::Local {
                    index: *index,
                    input: input.clone(),
                })
            }
            CellState::Running { .. } | CellState::Completed { .. } => None,
            CellState::Ready { .. } => None,
        }
    }

    pub(crate) fn start(
        &mut self,
        index: usize,
        step: &StepIdentity,
        input: EventPayload,
    ) -> Result<(), JournalError> {
        let checkpoint = match &self.state {
            CellState::Ready {
                index: expected,
                input: expected_input,
                checkpoint,
                ..
            } if *expected == index && *expected_input == input => checkpoint.clone(),
            _ => return Err(invalid_state("invalid component start transition")),
        };
        // provably identical to the state's current input by the guard above -- never worth
        // re-journaling a potentially large payload a second time.
        self.journal.append(&JournalEvent::ComponentStarted {
            index,
            name: step.name.clone(),
            hash: step.hash.to_string(),
            input: EventPayload::Reuse,
            durable_after: Some(step.durable_after),
        })?;
        self.state = CellState::Running {
            index,
            input,
            checkpoint,
        };
        Ok(())
    }

    pub(crate) fn complete_component(
        &mut self,
        index: usize,
        output: EventPayload,
        duration_us: u64,
        durable_after: bool,
    ) -> Result<(), JournalError> {
        let input = match &self.state {
            CellState::Running {
                index: expected,
                input,
                ..
            } if *expected == index => input.clone(),
            _ => return Err(invalid_state("invalid component completion transition")),
        };
        self.journal.append(&JournalEvent::ComponentCompleted {
            index,
            output: output.clone(),
            duration_us: Some(duration_us),
        })?;
        let next = index
            .checked_add(1)
            .ok_or_else(|| invalid_state("component index overflow"))?;
        self.state = CellState::Ready {
            index: next,
            input: output,
            checkpoint: None,
            retry: durable_after.then_some((index, input)),
        };
        Ok(())
    }

    pub(crate) fn checkpoint(
        &mut self,
        index: usize,
        hash: String,
        backend: String,
        bytes: u64,
        duration_us: u64,
    ) -> Result<(), JournalError> {
        match &self.state {
            CellState::Ready {
                index: next,
                checkpoint: None,
                retry: Some((completed, _)),
                ..
            } if *completed == index
                && *next
                    == index
                        .checked_add(1)
                        .ok_or_else(|| invalid_state("component index overflow"))? => {}
            _ => return Err(invalid_state("invalid checkpoint transition")),
        }
        self.journal.append(&JournalEvent::CheckpointCreated {
            index,
            hash: hash.clone(),
            backend: Some(backend.clone()),
            bytes: Some(bytes),
            duration_us: Some(duration_us),
        })?;
        if let CellState::Ready {
            checkpoint, retry, ..
        } = &mut self.state
        {
            *checkpoint = Some(Checkpoint {
                hash,
                backend: Some(backend),
            });
            *retry = None;
        }
        Ok(())
    }

    pub(crate) fn finish(&mut self, step_count: usize) -> Result<EventPayload, JournalError> {
        match &self.state {
            CellState::Completed { output } => Ok(output.clone()),
            CellState::Ready {
                index,
                input,
                checkpoint: None,
                retry: None,
            } if *index == step_count => {
                let output = input.clone();
                // provably identical to the state's current input -- see `start()`.
                self.journal.append(&JournalEvent::WorkflowCompleted {
                    output: EventPayload::Reuse,
                })?;
                self.state = CellState::Completed {
                    output: output.clone(),
                };
                Ok(output)
            }
            _ => Err(invalid_state(
                "workflow cannot complete in its current state",
            )),
        }
    }

    pub(crate) fn resumed(&self) -> bool {
        self.resumed
    }
}

fn corrupt(sequence: i64, message: impl Into<String>) -> JournalError {
    JournalError::Corrupt {
        sequence,
        message: message.into(),
    }
}

fn invalid_state(message: impl Into<String>) -> JournalError {
    JournalError::InvalidState {
        message: message.into(),
    }
}
