use std::{path::Path, time::Instant};

use kairo_core::{ComponentHash, Durability, Workflow, WorkflowMode, WorkflowResources};
use kairo_storage::ArtifactStore;

use super::{
    CallKind, Result, Runtime, RuntimeError, StoreState, identity::StepIdentity,
    journal::JournalError,
};

mod component {
    wasmtime::component::bindgen!({
        world: "stage",
        path: "../../wit",
    });
}

mod durability;
mod execution;
mod groups;

use execution::CoreOutcome;

pub use groups::GroupOutcome;

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

fn step_identities(
    workflow: &Workflow,
    prepared: &[PreparedStep],
    resolved: &std::collections::HashMap<usize, bool>,
) -> Vec<StepIdentity> {
    prepared
        .iter()
        .enumerate()
        .map(|(index, step)| StepIdentity {
            name: step.name.clone(),
            hash: step.hash,
            durable_after: match workflow.durability_after_step(index) {
                Durability::Required => true,
                Durability::Ephemeral => false,
                Durability::Auto => resolved.get(&index).copied().unwrap_or(false),
            },
        })
        .collect()
}

impl Runtime {
    pub fn load_workflow(&self, path: impl AsRef<Path>) -> Result<Workflow> {
        let path = path.as_ref();
        let workflow = Workflow::load(
            path,
            self.config.max_workflow_bytes,
            self.config.max_workflow_steps,
        )
        .map_err(|source| RuntimeError::LoadWorkflow {
            path: path.to_path_buf(),
            source,
        })?;
        self.validate_workflow_resources(&workflow)?;
        Ok(workflow)
    }

    pub async fn run_workflow(&self, workflow: &Workflow) -> Result<WorkflowResult> {
        let prepared = self.prepare_workflow(workflow)?;

        let started = Instant::now();
        let mut output = workflow
            .scalar_input()
            .ok_or(RuntimeError::InvalidScalarWorkflowInput)?;
        for step in &prepared {
            output = self
                .run_workflow_step(step, output, workflow.resources())
                .await?
                .output;
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
        let state_path = state_path.as_ref();
        let (resolved, auto_plan) = self.resolve_durability(workflow, &prepared, state_path)?;
        if resolved.values().any(|&required| required) && artifacts.is_none() {
            return Err(RuntimeError::ArtifactStoreRequired);
        }
        let identities = step_identities(workflow, &prepared, &resolved);
        match self
            .run_cell_core(
                workflow,
                &prepared,
                &identities,
                &auto_plan,
                state_path,
                artifacts,
                input,
                0,
                input,
                pause_after,
            )
            .await?
        {
            CoreOutcome::Completed(result) => Ok(CellRunResult::Completed(result)),
            CoreOutcome::Paused { result, .. } => Ok(CellRunResult::Paused(result)),
        }
    }

    pub fn validate_workflow(&self, workflow: &Workflow) -> Result<()> {
        self.validate_workflow_resources(workflow)?;
        match workflow.mode() {
            WorkflowMode::Scalar => self.prepare_workflow(workflow).map(|_| ()),
            WorkflowMode::Stream => self.validate_stream_workflow(workflow),
        }
    }

    fn prepare_workflow(&self, workflow: &Workflow) -> Result<Vec<PreparedStep>> {
        self.validate_workflow_resources(workflow)?;
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

    async fn run_workflow_step(
        &self,
        step: &PreparedStep,
        input: u32,
        resources: Option<WorkflowResources>,
    ) -> Result<StepResult> {
        let mut store = self.new_store(resources)?;
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

    pub(crate) fn validate_workflow_resources(&self, workflow: &Workflow) -> Result<()> {
        let Some(resources) = workflow.resources() else {
            return Ok(());
        };
        if resources.fuel > self.config.max_workflow_fuel {
            return Err(RuntimeError::WorkflowFuelLimit {
                requested: resources.fuel,
                maximum: self.config.max_workflow_fuel,
            });
        }
        if resources.memory_bytes > self.config.max_workflow_memory_bytes {
            return Err(RuntimeError::WorkflowMemoryLimit {
                requested: resources.memory_bytes,
                maximum: self.config.max_workflow_memory_bytes,
            });
        }
        Ok(())
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
