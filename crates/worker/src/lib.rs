use std::time::{SystemTime, UNIX_EPOCH};

use kairo_control::{
    ControlError, Endpoint, RunRequest, WaitRequest, WorkerResult, worker_loop_with_waits,
};
use kairo_core::{Config, Workflow, WorkflowWait};
use kairo_runtime::{
    CellRunResult, DurableWait, Runtime, complete_workflow_wait, inspect_workflow_wait,
    record_workflow_wait,
};
use kairo_storage::ArtifactStore;

mod effect;

pub fn run(endpoint: Endpoint, worker: String, allow_console: bool) -> Result<(), ControlError> {
    worker_loop_with_waits(endpoint, worker, move |run| execute(run, allow_console))
}

fn execute(run: RunRequest, allow_console: bool) -> Result<WorkerResult, String> {
    let config = Config {
        allow_console,
        ..Config::default()
    };
    let runtime = Runtime::new(config).map_err(|error| error.to_string())?;
    let workflow = runtime
        .load_workflow(&run.workflow)
        .map_err(|error| error.to_string())?;
    let artifacts = run
        .storage
        .clone()
        .map(ArtifactStore::from_config)
        .transpose()
        .map_err(|error| error.to_string())?;
    let executor = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| error.to_string())?;
    let wait_after = boundary_index(&workflow, workflow.wait_after())?;
    let effect_after = boundary_index(
        &workflow,
        workflow
            .effect()
            .and_then(kairo_core::WorkflowEffect::after),
    )?;
    let mut wait_done = wait_after.is_none();
    if let Some(recorded) = inspect_workflow_wait(&run.state).map_err(string)? {
        if Some(recorded.after_step) != wait_after {
            return Err("durable wait does not match this workflow".to_owned());
        }
        if recorded.completed {
            wait_done = true;
        } else if run.wait.is_some() {
            complete_workflow_wait(&run.state, recorded.after_step).map_err(string)?;
            wait_done = true;
        } else {
            return Ok(WorkerResult::Waiting(control_wait(&recorded.wait)));
        }
    }
    let mut effect_done = effect_after.is_none();
    loop {
        let pause_after = [
            (!wait_done).then_some(wait_after).flatten(),
            (!effect_done).then_some(effect_after).flatten(),
        ]
        .into_iter()
        .flatten()
        .min();
        match executor
            .block_on(runtime.run_cell_until(
                &workflow,
                &run.state,
                artifacts.as_ref(),
                pause_after,
            ))
            .map_err(string)?
        {
            CellRunResult::Completed(result) => {
                if let Some(declaration) =
                    workflow.effect().filter(|effect| effect.after().is_none())
                {
                    effect::apply(&run, declaration.operation(), result.output)?;
                }
                return Ok(WorkerResult::Completed(result.output));
            }
            CellRunResult::Paused(_) if pause_after == wait_after && !wait_done => {
                let wait = durable_wait(
                    workflow
                        .wait()
                        .ok_or_else(|| "workflow wait boundary is missing".to_owned())?,
                )?;
                let recorded = record_workflow_wait(
                    &run.state,
                    wait_after.ok_or_else(|| "workflow wait boundary is missing".to_owned())?,
                    &wait,
                )
                .map_err(string)?;
                if recorded.completed {
                    wait_done = true;
                    continue;
                }
                return Ok(WorkerResult::Waiting(control_wait(&recorded.wait)));
            }
            CellRunResult::Paused(result) if pause_after == effect_after && !effect_done => {
                let declaration = workflow
                    .effect()
                    .ok_or_else(|| "workflow effect boundary is missing".to_owned())?;
                effect::apply(&run, declaration.operation(), result.output)?;
                effect_done = true;
            }
            CellRunResult::Paused(_) => {
                return Err("workflow paused at an invalid boundary".to_owned());
            }
        }
    }
}

fn boundary_index(
    workflow: &Workflow,
    boundary: Option<&kairo_core::ComponentId>,
) -> Result<Option<usize>, String> {
    boundary
        .map(|boundary| {
            workflow
                .steps()
                .iter()
                .position(|step| &step.id == boundary)
                .ok_or_else(|| format!("workflow boundary `{boundary}` is missing"))
        })
        .transpose()
}

fn durable_wait(wait: &WorkflowWait) -> Result<DurableWait, String> {
    match wait {
        WorkflowWait::Timer(duration) => {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(string)?
                .as_millis();
            Ok(DurableWait::Timer {
                due_ms: now
                    .saturating_add(duration.as_millis())
                    .min(u128::from(u64::MAX)) as u64,
            })
        }
        WorkflowWait::Signal(name) => Ok(DurableWait::Signal { name: name.clone() }),
    }
}

fn control_wait(wait: &DurableWait) -> WaitRequest {
    match wait {
        DurableWait::Timer { due_ms } => WaitRequest::Timer { due_ms: *due_ms },
        DurableWait::Signal { name } => WaitRequest::Signal { name: name.clone() },
    }
}

fn string(error: impl std::fmt::Display) -> String {
    error.to_string()
}
