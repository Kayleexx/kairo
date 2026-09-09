use std::path::Path;

use kairo_core::{Config, Workflow, WorkflowMode};
use kairo_runtime::Runtime;

use crate::{CliError, Result, discovery, lifecycle, service, setup, state, status, stream};

pub(crate) struct RunOptions<'a> {
    pub(crate) input: Option<u32>,
    pub(crate) input_file: Option<&'a Path>,
    pub(crate) materialize: bool,
    pub(crate) state_path: Option<&'a Path>,
    pub(crate) cell: Option<&'a str>,
    pub(crate) watch: bool,
    pub(crate) workers: Option<usize>,
}

pub(crate) async fn run_path(path: &Path, options: RunOptions<'_>, config: Config) -> Result<()> {
    let path = discovery::resolve(path, config).map_err(CliError::Discovery)?;
    if is_workflow(&path) {
        run_workflow(&path, options, config).await
    } else {
        if options.input_file.is_some() {
            return Err(CliError::StreamInput);
        }
        if options.materialize {
            return Err(CliError::Materialize);
        }
        if options.state_path.is_some() || options.cell.is_some() {
            return Err(CliError::State);
        }
        if options.watch {
            return Err(CliError::Watch);
        }
        run_component(&path, options.input.unwrap_or_default(), config).await
    }
}

pub(crate) async fn run_component(path: &Path, input: u32, config: Config) -> Result<()> {
    status("36", "→", &format!("running {}", path.display()));
    let runtime = Runtime::new(config)?;
    let component = runtime.load_component(path)?;
    let result = runtime.run_component(&component, input).await?;
    status("32", "✓", &format!("completed in {:?}", result.duration));
    println!("{}", result.output);
    Ok(())
}

async fn run_workflow(path: &Path, options: RunOptions<'_>, config: Config) -> Result<()> {
    let runtime = Runtime::new(config)?;
    let workflow = runtime.load_workflow(path)?;
    match workflow.mode() {
        WorkflowMode::Scalar => {
            if options.input.is_some() {
                return Err(CliError::WorkflowInput);
            }
            if options.input_file.is_some() {
                return Err(CliError::StreamInput);
            }
            if options.materialize {
                return Err(CliError::Materialize);
            }
            let mut state_path =
                state::resolve_run(options.state_path, options.cell, workflow.name())?;
            if state_path.is_none()
                && (options.watch
                    || kairo_control::load_endpoint(Path::new(".kairo")).is_ok()
                    || workflow.requires_durable_artifacts()
                    || workflow.wait().is_some()
                    || workflow.effect().is_some())
            {
                state_path = Some(state::generated_run(workflow.name()));
            }
            if workflow.requires_durable_artifacts() && setup::ensure_storage()? {
                status("32", "✓", "local artifact storage ready");
            }
            if options.watch {
                return service::watch_run(options.workers, &workflow, path, state_path.as_deref());
            }
            if (workflow.wait().is_some() || workflow.effect().is_some())
                && kairo_control::load_endpoint(Path::new(".kairo")).is_err()
            {
                lifecycle::start(2, false)?;
            }
            run_scalar_workflow(&runtime, &workflow, path, state_path.as_deref()).await
        }
        WorkflowMode::Stream => {
            if options.input.is_some() {
                return Err(CliError::WorkflowInput);
            }
            if options.state_path.is_some() || options.cell.is_some() {
                return Err(CliError::State);
            }
            if options.watch {
                return Err(CliError::Watch);
            }
            stream::run(&runtime, &workflow, options.input_file, options.materialize).await
        }
    }
}

async fn run_scalar_workflow(
    runtime: &Runtime,
    workflow: &Workflow,
    workflow_path: &Path,
    state_path: Option<&Path>,
) -> Result<()> {
    if let (Ok(endpoint), Some(state_path)) = (
        kairo_control::load_endpoint(Path::new(".kairo")),
        state_path,
    ) {
        match service::submit_run(&endpoint, workflow, workflow_path, state_path) {
            Ok(()) => return Ok(()),
            Err(CliError::Control(kairo_control::ControlError::Unavailable)) => {}
            Err(error) => return Err(error),
        }
    }
    if workflow.effect().is_some() {
        return Err(CliError::Effect(
            "effect workflows need local services; run `kairo effects serve`, then `kairo start`"
                .to_owned(),
        ));
    }
    status("36", "→", workflow.name());
    let artifacts = workflow
        .requires_durable_artifacts()
        .then(setup::artifact_store)
        .transpose()?;
    let result = match state_path {
        Some(path) => runtime.run_cell(workflow, path, artifacts.as_ref()).await?,
        None => runtime.run_workflow(workflow).await?,
    };
    status(
        "32",
        "✓",
        &format!(
            "{} {} in {:?}",
            if result.resumed {
                "restored from journal"
            } else {
                "completed"
            },
            workflow.name(),
            result.duration
        ),
    );
    println!("{}", result.output);
    Ok(())
}

fn is_workflow(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("yaml") || extension.eq_ignore_ascii_case("yml")
        })
}
