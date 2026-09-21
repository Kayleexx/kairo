use std::path::Path;

use kairo_core::{Config, Durability, Workflow, WorkflowMode};
use kairo_runtime::Runtime;

use crate::{CliError, Result, discovery, is_workflow, service, setup, state, status, stream};

mod value;

pub(crate) struct RunOptions<'a> {
    pub(crate) input: Option<u32>,
    pub(crate) input_file: Option<&'a Path>,
    /// the bare `kairo run <workflow> <input>` argument, whose meaning depends on the workflow's
    /// declared input mode -- resolved once the workflow is loaded, not here.
    pub(crate) positional: Option<&'a Path>,
    pub(crate) value: Option<&'a str>,
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
        if options.input_file.is_some() || options.positional.is_some() {
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
            if options.positional.is_some() {
                return Err(CliError::UnexpectedInput {
                    workflow: workflow.name().to_owned(),
                });
            }
            if options.value.is_some() {
                return Err(CliError::ValueInput);
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
            if workflow.has_unresolved_durability() {
                ensure_profiled(&runtime, &workflow, path, None, config.allow_console)?;
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
            if options.value.is_some() {
                return Err(CliError::ValueInput);
            }
            if options.output.is_some() && workflow.output().is_none() {
                return Err(CliError::Output);
            }
            if options.no_export && workflow.output().is_none() {
                return Err(CliError::Output);
            }
            let input_file = options.input_file.or(options.positional);
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
            // stream workflows stay on their direct local path unless the caller explicitly asks
            // for a managed live run. A stale or unrelated control endpoint must never change a
            // normal `kairo run` into a worker submission.
            let managed = options.watch || options.workers.is_some();
            if managed {
                let state_path = state_path.as_deref().ok_or(CliError::State)?;
                let output = service::watch_stream_run(
                    options.workers,
                    &workflow,
                    path,
                    state_path,
                    input_file,
                    config.allow_console,
                    options.verbose,
                )?;
                status("32", "✓", &format!("completed {}", workflow.name()));
                println!("{output}");
                return Ok(());
            }
            stream::run(
                &runtime,
                &workflow,
                input_file,
                options.materialize,
                state_path.as_deref(),
                artifacts.as_ref(),
                options.output,
                options.no_export,
            )
            .await
        }
        WorkflowMode::Value => value::run(&runtime, &workflow, path, options, config).await,
    }
}

/// runs the smallest real measurement Kairo needs before it can honor a fresh `durability: auto`
/// edge, then caches it -- a user never has to run a separate profiling command first. A no-op
/// once a *trusted* measurement already exists for this exact workflow shape -- a profile with
/// fewer than `MIN_TRUSTED_SAMPLES` real samples still gets one more top-up measurement, since
/// ordinary runs keep every profile improving from here on (`record_profile_observations`), this
/// is only ever needed for a genuinely new or rarely-run shape.
pub(super) fn ensure_profiled(
    runtime: &Runtime,
    workflow: &Workflow,
    path: &Path,
    value: Option<&str>,
    allow_console: bool,
) -> Result<()> {
    let Some(index) = (0..workflow.steps().len().saturating_sub(1))
        .find(|&index| workflow.durability_after_step(index) == Durability::Auto)
    else {
        return Ok(());
    };
    let trusted = runtime
        .auto_edge_profile_samples(workflow, index)?
        .is_some_and(|samples| samples >= kairo_runtime::MIN_TRUSTED_SAMPLES);
    if trusted {
        return Ok(());
    }
    status(
        "36",
        "→",
        &format!("measuring {} (first run)", workflow.name()),
    );
    crate::bench::quick_profile(path, value, allow_console)?;
    Ok(())
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
