use kairo_core::ComponentHash;
use sha2::{Digest, Sha256};

use crate::{
    journal::{Journal, JournalError},
    journal_event::JournalEvent,
};

pub(crate) struct StepIdentity {
    pub(crate) name: String,
    pub(crate) hash: ComponentHash,
}

enum CellState {
    Ready { index: usize, input: u32 },
    Running { index: usize, input: u32 },
    Completed { output: u32 },
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
        input: u32,
        steps: &[StepIdentity],
    ) -> Result<Self, JournalError> {
        let fingerprint = workflow_fingerprint(workflow_name, input, steps);
        let mut state = None;
        let found = journal.replay(|sequence, event| {
            apply_event(sequence, event, &fingerprint, input, steps, &mut state)
        })?;

        if !found {
            journal.append(&JournalEvent::WorkflowStarted { fingerprint, input })?;
            state = Some(CellState::Ready { index: 0, input });
        }

        let state = match state.ok_or_else(|| corrupt(0, "journal has no workflow start"))? {
            CellState::Running { index, input } => CellState::Ready { index, input },
            state => state,
        };
        Ok(Self {
            journal,
            state,
            resumed: found,
        })
    }

    pub(crate) fn next(&self, step_count: usize) -> Option<(usize, u32)> {
        match self.state {
            CellState::Ready { index, input } if index < step_count => Some((index, input)),
            CellState::Running { .. } | CellState::Completed { .. } => None,
            CellState::Ready { .. } => None,
        }
    }

    pub(crate) fn start(
        &mut self,
        index: usize,
        step: &StepIdentity,
        input: u32,
    ) -> Result<(), JournalError> {
        match self.state {
            CellState::Ready {
                index: expected,
                input: expected_input,
            } if expected == index && expected_input == input => {}
            _ => return Err(invalid_state("invalid component start transition")),
        }
        self.journal.append(&JournalEvent::ComponentStarted {
            index,
            name: step.name.clone(),
            hash: step.hash.to_string(),
            input,
        })?;
        self.state = CellState::Running { index, input };
        Ok(())
    }

    pub(crate) fn complete_component(
        &mut self,
        index: usize,
        output: u32,
    ) -> Result<(), JournalError> {
        match self.state {
            CellState::Running {
                index: expected, ..
            } if expected == index => {}
            _ => return Err(invalid_state("invalid component completion transition")),
        }
        self.journal
            .append(&JournalEvent::ComponentCompleted { index, output })?;
        let next = index
            .checked_add(1)
            .ok_or_else(|| invalid_state("component index overflow"))?;
        self.state = CellState::Ready {
            index: next,
            input: output,
        };
        Ok(())
    }

    pub(crate) fn finish(&mut self, step_count: usize) -> Result<u32, JournalError> {
        match self.state {
            CellState::Completed { output } => Ok(output),
            CellState::Ready { index, input } if index == step_count => {
                self.journal
                    .append(&JournalEvent::WorkflowCompleted { output: input })?;
                self.state = CellState::Completed { output: input };
                Ok(input)
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

fn apply_event(
    sequence: i64,
    event: JournalEvent,
    fingerprint: &str,
    workflow_input: u32,
    steps: &[StepIdentity],
    state: &mut Option<CellState>,
) -> Result<(), JournalError> {
    match event {
        JournalEvent::WorkflowStarted {
            fingerprint: stored,
            input,
        } => {
            if state.is_some() {
                return Err(corrupt(sequence, "duplicate workflow start"));
            }
            if stored != fingerprint || input != workflow_input {
                return Err(corrupt(
                    sequence,
                    "workflow does not match the recorded execution",
                ));
            }
            *state = Some(CellState::Ready { index: 0, input });
        }
        JournalEvent::ComponentStarted {
            index,
            name,
            hash,
            input,
        } => {
            let step = steps
                .get(index)
                .ok_or_else(|| corrupt(sequence, "component index is out of bounds"))?;
            if step.name != name || step.hash.to_string() != hash {
                return Err(corrupt(sequence, "component identity does not match"));
            }
            match state {
                Some(CellState::Ready {
                    index: expected,
                    input: expected_input,
                }) if *expected == index && *expected_input == input => {}
                Some(CellState::Running {
                    index: expected,
                    input: expected_input,
                }) if *expected == index && *expected_input == input => {}
                _ => return Err(corrupt(sequence, "unexpected component start")),
            }
            *state = Some(CellState::Running { index, input });
        }
        JournalEvent::ComponentCompleted { index, output } => match state {
            Some(CellState::Running {
                index: expected, ..
            }) if *expected == index => {
                let next = index
                    .checked_add(1)
                    .ok_or_else(|| corrupt(sequence, "component index overflow"))?;
                *state = Some(CellState::Ready {
                    index: next,
                    input: output,
                });
            }
            _ => return Err(corrupt(sequence, "unexpected component completion")),
        },
        JournalEvent::WorkflowCompleted { output } => match state {
            Some(CellState::Ready { index, input })
                if *index == steps.len() && *input == output =>
            {
                *state = Some(CellState::Completed { output });
            }
            _ => return Err(corrupt(sequence, "unexpected workflow completion")),
        },
    }
    Ok(())
}

fn workflow_fingerprint(name: &str, input: u32, steps: &[StepIdentity]) -> String {
    let mut digest = Sha256::new();
    hash_part(&mut digest, name.as_bytes());
    hash_part(&mut digest, &input.to_le_bytes());
    for step in steps {
        hash_part(&mut digest, step.name.as_bytes());
        hash_part(&mut digest, step.hash.to_string().as_bytes());
    }
    format!("{:x}", digest.finalize())
}

fn hash_part(digest: &mut Sha256, value: &[u8]) {
    digest.update(value.len().to_le_bytes());
    digest.update(value);
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
