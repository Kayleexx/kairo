use std::{path::Path, time::Instant};

use kairo_core::{ComponentHash, Durability, Workflow, WorkflowMode};
use kairo_storage::ArtifactStore;

use super::{
    CallKind, Result, Runtime, RuntimeError, StoreState,
    cell::{Cell, PendingStep},
    identity::StepIdentity,
    journal::{Journal, JournalError},
};

mod component {
    wasmtime::component::bindgen!({
        world: "stage",
        path: "../../wit",
    });
}

struct PreparedStep {
    name: String,
    hash: ComponentHash,
    stage: component::StagePre<StoreState>,
}

struct StepResult {
    output: u32,
    duration: std::time::Duration,
}

#[derive(Clone, Copy, Debug)]
pub struct WorkflowResult {
    pub output: u32,
    pub duration: std::time::Duration,
    pub resumed: bool,
}

#[derive(Clone, Copy, Debug)]
pub enum CellRunResult {
    Completed(WorkflowResult),
    Paused(WorkflowResult),
}

impl Runtime {
    pub fn load_workflow(&self, path: impl AsRef<Path>) -> Result<Workflow> {
        let path = path.as_ref();
        Workflow::load(
            path,
            self.config.max_workflow_bytes,
            self.config.max_workflow_steps,
        )
        .map_err(|source| RuntimeError::LoadWorkflow {
            path: path.to_path_buf(),
            source,
        })
    }

    pub async fn run_workflow(&self, workflow: &Workflow) -> Result<WorkflowResult> {
        let prepared = self.prepare_workflow(workflow)?;

        let started = Instant::now();
        let mut output = workflow
            .scalar_input()
            .ok_or(RuntimeError::InvalidScalarWorkflowInput)?;
        for step in &prepared {
            output = self.run_workflow_step(step, output).await?.output;
        }
        let duration = started.elapsed();
        tracing::info!(
            workflow = workflow.name(),
            duration_us = duration.as_micros(),
            output,
            "workflow executed"
        );
        Ok(WorkflowResult {
            output,
            duration,
            resumed: false,
        })
    }

    pub async fn run_cell(
        &self,
        workflow: &Workflow,
        state_path: impl AsRef<Path>,
        artifacts: Option<&ArtifactStore>,
    ) -> Result<WorkflowResult> {
        match self
            .run_cell_until(workflow, state_path, artifacts, None)
            .await?
        {
            CellRunResult::Completed(result) => Ok(result),
            CellRunResult::Paused(_) => Err(RuntimeError::UnexpectedPause),
        }
    }

    pub async fn run_cell_until(
        &self,
        workflow: &Workflow,
        state_path: impl AsRef<Path>,
        artifacts: Option<&ArtifactStore>,
        pause_after: Option<usize>,
    ) -> Result<CellRunResult> {
        if workflow.mode() != WorkflowMode::Scalar {
            return Err(RuntimeError::StatefulStreamWorkflow);
        }
        if workflow.requires_durable_artifacts() && artifacts.is_none() {
            return Err(RuntimeError::ArtifactStoreRequired);
        }
        let prepared = self.prepare_workflow(workflow)?;
        let input = workflow
            .scalar_input()
            .ok_or(RuntimeError::InvalidScalarWorkflowInput)?;
        let identities: Vec<_> = prepared
            .iter()
            .enumerate()
            .map(|(index, step)| StepIdentity {
                name: step.name.clone(),
                hash: step.hash,
                durable_after: workflow.durability_after_step(index) == Durability::Required,
            })
            .collect();
        let state_path = state_path.as_ref();
        let journal =
            Journal::open(state_path).map_err(|source| self.journal_error(state_path, source))?;
        let mut cell = Cell::open(journal, workflow.name(), input, &identities)
            .map_err(|source| self.journal_error(state_path, source))?;
        let resumed = cell.resumed();

        let started = Instant::now();
        while let Some(pending) = cell.next(prepared.len()) {
            let (index, input) = match pending {
                PendingStep::Local { index, input } => (index, input),
                PendingStep::Checkpoint {
                    index,
                    input,
                    hash,
                    backend,
                } => {
                    let artifacts = artifacts.ok_or(RuntimeError::ArtifactStoreRequired)?;
                    let configured = artifacts.backend().as_str();
                    if let Some(recorded) = backend.filter(|recorded| recorded != configured) {
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
                        return Ok(CellRunResult::Paused(WorkflowResult {
                            output: artifact.value,
                            duration: started.elapsed(),
                            resumed,
                        }));
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
            cell.start(index, identity, input)
                .map_err(|source| self.journal_error(state_path, source))?;
            tracing::info!(step = step.name, index, input, "cell component started");
            let result = self.run_workflow_step(step, input).await?;
            cell.complete_component(
                index,
                result.output,
                duration_us(result.duration),
                identity.durable_after,
            )
            .map_err(|source| self.journal_error(state_path, source))?;
            if identity.durable_after {
                let store = artifacts.ok_or(RuntimeError::ArtifactStoreRequired)?;
                let artifact = store
                    .put(result.output)
                    .await
                    .map_err(|source| RuntimeError::Artifact { source })?;
                cell.checkpoint(
                    index,
                    artifact.hash.clone(),
                    store.backend().as_str().to_owned(),
                )
                .map_err(|source| self.journal_error(state_path, source))?;
                tracing::info!(index, hash = artifact.hash, "checkpoint created");
                if pause_after == Some(index) {
                    return Ok(CellRunResult::Paused(WorkflowResult {
                        output: result.output,
                        duration: started.elapsed(),
                        resumed,
                    }));
                }
            }
        }
        let output = cell
            .finish(prepared.len())
            .map_err(|source| self.journal_error(state_path, source))?;
        let duration = started.elapsed();
        tracing::info!(
            workflow = workflow.name(),
            duration_us = duration.as_micros(),
            output,
            resumed,
            state = %state_path.display(),
            "cell executed"
        );
        Ok(CellRunResult::Completed(WorkflowResult {
            output,
            duration,
            resumed,
        }))
    }

    pub fn validate_workflow(&self, workflow: &Workflow) -> Result<()> {
        match workflow.mode() {
            WorkflowMode::Scalar => self.prepare_workflow(workflow).map(|_| ()),
            WorkflowMode::Stream => self.validate_stream_workflow(workflow),
        }
    }

    fn prepare_workflow(&self, workflow: &Workflow) -> Result<Vec<PreparedStep>> {
        let linker = self.component_linker()?;
        let mut prepared = Vec::with_capacity(workflow.steps().len());
        for step in workflow.steps() {
            let loaded = self
                .load_component(&step.component)
                .map_err(|source| self.workflow_step_error(step.id.as_str(), source))?;
            let pre = linker
                .instantiate_pre(&loaded.component)
                .map_err(|source| {
                    self.workflow_step_error(step.id.as_str(), RuntimeError::Instantiate { source })
                })?;
            let stage = component::StagePre::new(pre).map_err(|source| {
                RuntimeError::IncompatibleWorkflowComponent {
                    step: step.id.to_string(),
                    path: step.component.clone(),
                    source,
                }
            })?;
            prepared.push(PreparedStep {
                name: step.id.to_string(),
                hash: loaded.hash,
                stage,
            });
        }
        Ok(prepared)
    }

    async fn run_workflow_step(&self, step: &PreparedStep, input: u32) -> Result<StepResult> {
        let mut store = self.new_store()?;
        let stage = step
            .stage
            .instantiate_async(&mut store)
            .await
            .map_err(|source| {
                self.workflow_step_error(&step.name, self.instantiation_error(source, &store))
            })?;
        let started = Instant::now();
        let output = match store
            .run_concurrent(async |accessor| stage.call_run(accessor, input).await)
            .await
        {
            Ok(Ok(output)) => output,
            Ok(Err(source)) => {
                let error = self.execution_error(source, &store, CallKind::Workflow);
                return Err(self.workflow_step_error(&step.name, error));
            }
            Err(source) => {
                let error = self.execution_error(source, &store, CallKind::Runtime);
                return Err(self.workflow_step_error(&step.name, error));
            }
        };
        let duration = started.elapsed();
        tracing::info!(
            step = step.name,
            hash = %step.hash,
            duration_us = duration.as_micros(),
            input,
            output,
            "workflow step executed"
        );
        Ok(StepResult { output, duration })
    }

    fn workflow_step_error(&self, step: &str, source: RuntimeError) -> RuntimeError {
        RuntimeError::WorkflowStep {
            step: step.to_owned(),
            source: Box::new(source),
        }
    }

    fn journal_error(&self, path: &Path, source: JournalError) -> RuntimeError {
        RuntimeError::Journal {
            path: path.to_path_buf(),
            source,
        }
    }
}

fn duration_us(duration: std::time::Duration) -> u64 {
    duration.as_micros().min(u128::from(u64::MAX)) as u64
}
