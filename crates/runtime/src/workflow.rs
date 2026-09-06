use std::{path::Path, time::Instant};

use kairo_core::{ComponentHash, Workflow};

use super::{CallKind, Result, Runtime, RuntimeError, StoreState};

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
        let mut output = workflow.input();
        for step in prepared {
            output = self.run_workflow_step(step, output).await?;
        }
        let duration = started.elapsed();
        tracing::info!(
            workflow = workflow.name(),
            duration_us = duration.as_micros(),
            output,
            "workflow executed"
        );
        Ok(WorkflowResult { output, duration })
    }

    pub fn validate_workflow(&self, workflow: &Workflow) -> Result<()> {
        self.prepare_workflow(workflow).map(|_| ())
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

    async fn run_workflow_step(&self, step: PreparedStep, input: u32) -> Result<u32> {
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
}
