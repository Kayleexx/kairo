use std::path::Path;

use kairo_core::{Config, Workflow, WorkflowMode};
use kairo_runtime::Runtime;

use crate::{CliError, Result, discovery, is_workflow, service, setup, state, status, stream};

mod value;

pub(crate) struct RunOptions<'a> {
    pub(crate) input: Option<u32>,
    pub(crate) input_file: Option<&'a Path>,
    pub(crate) output: Option<&'a Path>,
    pub(crate) no_export: bool,
    pub(crate) materialize: bool,
    pub(crate) state_path: Option<&'a Path>,
    pub(crate) cell: Option<&'a str>,
    pub(crate) watch: bool,
    pub(crate) workers: Option<usize>,
    pub(crate) verbose: bool,
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
        if options.output.is_some() {
            return Err(CliError::Output);
        }
        if options.no_export {
            return Err(CliError::Output);
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
            if options.output.is_some() {
                return Err(CliError::Output);
            }
            if options.no_export {
                return Err(CliError::Output);
            }
            let mut state_path =
                state::resolve_run(options.state_path, options.cell, workflow.name())?;
            if state_path.is_none()
                && (options.watch
                    || kairo_control::load_endpoint(Path::new(".kairo")).is_ok()
                    || workflow.requires_durable_artifacts()
                    || workflow.has_unresolved_durability()
                    || workflow.wait().is_some()
                    || workflow.effect().is_some())
            {
                state_path = Some(state::generated_run(workflow.name())?);
            }
            if (workflow.requires_durable_artifacts() || workflow.has_unresolved_durability())
                && setup::ensure_storage()?
            {
                status("32", "✓", "local artifact storage ready");
            }
            if options.watch {
                return service::watch_run(
                    options.workers,
                    &workflow,
                    path,
                    state_path.as_deref(),
                    config.allow_console,
                    options.verbose,
                );
            }
            run_scalar_workflow(
                &runtime,
                &workflow,
                path,
                state_path.as_deref(),
                config.allow_console,
                options.verbose,
            )
            .await
        }
        WorkflowMode::Stream => {
            if options.input.is_some() {
                return Err(CliError::WorkflowInput);
            }
            if options.workers.is_some() {
                return Err(CliError::StreamWorkers);
            }
            if options.output.is_some() && workflow.output().is_none() {
                return Err(CliError::Output);
            }
            if options.no_export && workflow.output().is_none() {
                return Err(CliError::Output);
            }
            let artifacts = if workflow.output().is_some() {
                if setup::ensure_storage()? {
                    status("32", "✓", "local artifact storage ready");
                }
                Some(setup::artifact_store()?)
            } else {
                None
            };
            let mut state_path =
                state::resolve_run(options.state_path, options.cell, workflow.name())?;
            if state_path.is_none() {
                state_path = Some(state::generated_run(workflow.name())?);
            }
            stream::run(
                &runtime,
                &workflow,
                options.input_file,
                options.materialize,
                state_path.as_deref(),
                artifacts.as_ref(),
                options.output,
                options.no_export,
            )
            .await
        }
        WorkflowMode::Value => value::run(&runtime, &workflow, options, config).await,
    }
}

// mirrors `has_unresolved_durability()`'s own conservative treatment of `auto` edges: an
// unresolved edge might still become `required`, so it counts here too.
fn needs_managed_execution(workflow: &Workflow) -> bool {
    workflow.requires_durable_artifacts()
        || workflow.has_unresolved_durability()
        || workflow.wait().is_some()
        || workflow.effect().is_some()
}

async fn run_scalar_workflow(
    runtime: &Runtime,
    workflow: &Workflow,
    workflow_path: &Path,
    state_path: Option<&Path>,
    allow_console: bool,
    verbose: bool,
) -> Result<()> {
    let has_endpoint = kairo_control::load_endpoint(Path::new(".kairo")).is_ok();
    if let Some(state_path) =
        state_path.filter(|_| has_endpoint || needs_managed_execution(workflow))
    {
        let (endpoint, mut local) = service::ensure_endpoint(None, allow_console, verbose)?;
        let result = service::submit_run(&endpoint, workflow, workflow_path, state_path);
        if let Some(local) = &mut local {
            local.stop()?;
        }
        return result;
    }
    status("36", "→", workflow.name());
    let artifacts = (workflow.requires_durable_artifacts() || workflow.has_unresolved_durability())
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
