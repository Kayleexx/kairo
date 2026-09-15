use std::{path::Path, time::Instant};

use kairo_core::Workflow;
use kairo_storage::ArtifactStore;

use super::{PreparedStep, WorkflowResult, duration_us};
use crate::{
    Result, Runtime, RuntimeError,
    cell::{Cell, PendingStep},
    durability_plan::AutoResolution,
    identity::StepIdentity,
    journal::{Journal, JournalError},
    payload::EventPayload,
};

pub(super) enum CoreOutcome {
    Completed(WorkflowResult),
    Paused {
        result: WorkflowResult,
        hash: String,
        backend: String,
    },
}

impl Runtime {
    /// Drives a Cell's journal from `start_index`/`start_input` through `prepared`'s steps,
    /// stopping (without executing the next step) right after the checkpoint at `pause_after` is
    /// committed, or running to actual workflow completion when `pause_after` is `None`. Shared by
    /// the whole-workflow path (`run_cell_until`, always `start_index: 0`) and the ExecutionGroup
    /// path (`run_cell_group`, any group's own start position). Scalar-only: `WorkflowResult`'s
    /// `u32` output is relied on by the worker/effect/CLI paths, so this stays a dedicated path
    /// rather than a payload-generic one -- mirrors how stream mode already has its own execution
    /// module instead of sharing this one.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn run_cell_core(
        &self,
        workflow: &Workflow,
        prepared: &[PreparedStep],
        identities: &[StepIdentity],
        auto_plan: &[AutoResolution],
        state_path: &Path,
        artifacts: Option<&ArtifactStore>,
        fingerprint_input: u32,
        start_index: usize,
        start_input: u32,
        pause_after: Option<usize>,
    ) -> Result<CoreOutcome> {
        let journal =
            Journal::open(state_path).map_err(|source| self.journal_error(state_path, source))?;
        let mut cell = Cell::open(
            journal,
            workflow.name(),
            EventPayload::Scalar(fingerprint_input),
            identities,
            auto_plan,
            start_index,
            EventPayload::Scalar(start_input),
        )
        .map_err(|source| self.journal_error(state_path, source))?;
        let resumed = cell.resumed();

        let started = Instant::now();
        while let Some(pending) = cell.next(prepared.len()) {
            let (index, input) = match pending {
                PendingStep::Local { index, input } => {
                    (index, self.scalar_payload(state_path, input)?)
                }
                PendingStep::Checkpoint {
                    index,
                    input,
                    hash,
                    backend,
                } => {
                    let input = self.scalar_payload(state_path, input)?;
                    let artifacts = artifacts.ok_or(RuntimeError::ArtifactStoreRequired)?;
                    let configured = artifacts.backend().as_str();
                    if let Some(recorded) =
                        backend.clone().filter(|recorded| recorded != configured)
                    {
                        return Err(RuntimeError::CheckpointBackendMismatch {
                            hash,
                            recorded,
                            configured,
                        });
                    }
                    let artifact = artifacts
                        .get(&hash)
                        .await
                        .map_err(|source| RuntimeError::Artifact { source })?;
                    if artifact.value != input {
                        return Err(RuntimeError::CheckpointMismatch {
                            hash,
                            expected: input,
                            found: artifact.value,
                        });
                    }
                    tracing::info!(index, hash = artifact.hash, "checkpoint restored");
                    if pause_after.and_then(|step| step.checked_add(1)) == Some(index) {
                        return Ok(CoreOutcome::Paused {
                            result: WorkflowResult {
                                output: artifact.value,
                                duration: started.elapsed(),
                                resumed,
                            },
                            hash,
                            backend: backend.unwrap_or_else(|| configured.to_owned()),
                        });
                    }
                    (index, artifact.value)
                }
            };
            let (step, identity) =
                prepared
                    .get(index)
                    .zip(identities.get(index))
                    .ok_or_else(|| {
                        self.journal_error(
                            state_path,
                            JournalError::InvalidState {
                                message: "next component is out of bounds".to_owned(),
                            },
                        )
                    })?;
            cell.start(index, identity, EventPayload::Scalar(input))
                .map_err(|source| self.journal_error(state_path, source))?;
            tracing::info!(step = step.name, index, input, "cell component started");
            let result = self
                .run_workflow_step(step, input, workflow.resources())
                .await?;
            cell.complete_component(
                index,
                EventPayload::Scalar(result.output),
                duration_us(result.duration),
                identity.durable_after,
            )
            .map_err(|source| self.journal_error(state_path, source))?;
            if identity.durable_after {
                let store = artifacts.ok_or(RuntimeError::ArtifactStoreRequired)?;
                let checkpoint_started = std::time::Instant::now();
                let artifact = store
                    .put(result.output)
                    .await
                    .map_err(|source| RuntimeError::Artifact { source })?;
                cell.checkpoint(
                    index,
                    artifact.hash.clone(),
                    store.backend().as_str().to_owned(),
                    artifact.bytes,
                    duration_us(checkpoint_started.elapsed()),
                )
                .map_err(|source| self.journal_error(state_path, source))?;
                tracing::info!(index, hash = artifact.hash, "checkpoint created");
                if pause_after == Some(index) {
                    return Ok(CoreOutcome::Paused {
                        result: WorkflowResult {
                            output: result.output,
                            duration: started.elapsed(),
                            resumed,
                        },
                        hash: artifact.hash,
                        backend: store.backend().as_str().to_owned(),
                    });
                }
            }
        }
        let output = cell
            .finish(prepared.len())
            .map_err(|source| self.journal_error(state_path, source))?;
        let output = self.scalar_payload(state_path, output)?;
        let duration = started.elapsed();
        tracing::info!(
            workflow = workflow.name(),
            duration_us = duration.as_micros(),
            output,
            resumed,
            state = %state_path.display(),
            "cell executed"
        );
        Ok(CoreOutcome::Completed(WorkflowResult {
            output,
            duration,
            resumed,
        }))
    }
}
