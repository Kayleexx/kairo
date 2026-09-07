use std::{path::Path, time::Instant};

use kairo_core::{ComponentHash, Workflow, WorkflowMode};

use super::{
    CallKind, Result, Runtime, RuntimeError, StoreState,
    cell::{Cell, StepIdentity},
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

#[derive(Clone, Copy, Debug)]
pub struct WorkflowResult {
    pub output: u32,
    pub duration: std::time::Duration,
    pub resumed: bool,
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
            output = self.run_workflow_step(step, output).await?;
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
    ) -> Result<WorkflowResult> {
        if workflow.mode() != WorkflowMode::Scalar {
            return Err(RuntimeError::StatefulStreamWorkflow);
        }
        let prepared = self.prepare_workflow(workflow)?;
        let input = workflow
            .scalar_input()
            .ok_or(RuntimeError::InvalidScalarWorkflowInput)?;
        let identities: Vec<_> = prepared
            .iter()
            .map(|step| StepIdentity {
                name: step.name.clone(),
                hash: step.hash,
            })
            .collect();
        let state_path = state_path.as_ref();
        let journal =
            Journal::open(state_path).map_err(|source| self.journal_error(state_path, source))?;
        let mut cell = Cell::open(journal, workflow.name(), input, &identities)
            .map_err(|source| self.journal_error(state_path, source))?;
        let resumed = cell.resumed();

        let started = Instant::now();
        while let Some((index, input)) = cell.next(prepared.len()) {
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
            let output = self.run_workflow_step(step, input).await?;
            cell.complete_component(index, output)
                .map_err(|source| self.journal_error(state_path, source))?;
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
        Ok(WorkflowResult {
            output,
            duration,
            resumed,
        })
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

    async fn run_workflow_step(&self, step: &PreparedStep, input: u32) -> Result<u32> {
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
        tracing::info!(
            step = step.name,
            hash = %step.hash,
            duration_us = started.elapsed().as_micros(),
            input,
            output,
            "workflow step executed"
        );
        Ok(output)
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
