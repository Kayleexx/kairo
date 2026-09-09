use kairo_control::{ControlError, Endpoint, RunRequest, worker_loop};
use kairo_core::Config;
use kairo_runtime::Runtime;
use kairo_storage::ArtifactStore;

mod effect;

pub fn run(endpoint: Endpoint, worker: String) -> Result<(), ControlError> {
    worker_loop(endpoint, worker, execute)
}

fn execute(run: RunRequest) -> Result<u32, String> {
    let runtime = Runtime::new(Config::default()).map_err(|error| error.to_string())?;
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
    let result = executor
        .block_on(runtime.run_cell(&workflow, &run.state, artifacts.as_ref()))
        .map_err(|error| error.to_string())?;
    if let Some(declaration) = workflow.effect() {
        effect::apply(&run, declaration.operation(), result.output)?;
    }
    Ok(result.output)
}
