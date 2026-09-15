use std::path::Path;

use crate::{CliError, Result, setup, status};
use kairo_core::Workflow;

mod cancel;
mod signal;
mod watch;

pub(crate) use cancel::cancel;
pub(crate) use kairo_control::LocalService;
pub(crate) use signal::signal;

const DEFAULT_LOCAL_WORKERS: usize = 2;

pub(crate) fn submit_run(
    endpoint: &kairo_control::Endpoint,
    workflow: &Workflow,
    workflow_path: &Path,
    state: &Path,
) -> Result<()> {
    let _effect = kairo_control::ensure_effect_service(workflow)?;
    let id = submit(endpoint, workflow, workflow_path, state)?;
    status("36", "→", &format!("queued {}", workflow.name()));
    let Some(output) = wait_for_output(endpoint, &id, workflow.wait_after().is_some())? else {
        return Ok(());
    };
    print_effect(workflow, state)?;
    status("32", "✓", &format!("completed {}", workflow.name()));
    println!("{output}");
    Ok(())
}

pub(crate) fn watch_run(
    workers: Option<usize>,
    workflow: &Workflow,
    workflow_path: &Path,
    state: Option<&Path>,
    allow_console: bool,
    verbose: bool,
) -> Result<()> {
    let state = state.ok_or(kairo_control::ControlError::State)?;
    let _effect = kairo_control::ensure_effect_service(workflow)?;
    let (endpoint, mut local) = ensure_endpoint(workers, allow_console, verbose)?;
    let id = submit(&endpoint, workflow, workflow_path, state)?;
    status("36", "→", workflow.name());
    let output = watch::output(&endpoint, &id, state)?;
    print_effect(workflow, state)?;
    if let Some(local) = &mut local {
        local.stop()?;
    }
    status("32", "✓", &format!("completed {}", workflow.name()));
    println!("{output}");
    Ok(())
}

fn print_effect(workflow: &Workflow, state: &Path) -> Result<()> {
    if workflow.effect().is_none() {
        return Ok(());
    }
    let receipts = kairo_runtime::inspect_receipts(state)
        .map_err(|error| CliError::Effect(error.to_string()))?;
    for receipt in receipts {
        if receipt.reused {
            status("36", "↻", "existing external action reused");
        } else {
            status(
                "32",
                "✓",
                &format!("external action {} committed", receipt.operation),
            );
        }
    }
    Ok(())
}

// thin wrapper: the actual request-building and polling is shared with any other front end
// (e.g. the TUI) via `kairo_control::submit_run`/`await_run` -- this only adds the storage
// lookup (a CLI/project-config concern) and this crate's own `CliError` conversion.
pub(crate) fn submit(
    endpoint: &kairo_control::Endpoint,
    workflow: &Workflow,
    workflow_path: &Path,
    state: &Path,
) -> Result<String> {
    let storage = (workflow.requires_durable_artifacts() || workflow.has_unresolved_durability())
        .then(setup::storage_config)
        .transpose()?;
    Ok(kairo_control::submit_run(
        endpoint,
        workflow,
        workflow_path,
        state,
        storage,
    )?)
}

fn wait_for_output(
    endpoint: &kairo_control::Endpoint,
    id: &str,
    detach_on_wait: bool,
) -> Result<Option<u32>> {
    match kairo_control::await_run(endpoint, id, detach_on_wait)? {
        kairo_control::SubmissionOutcome::Completed(output) => Ok(Some(output)),
        kairo_control::SubmissionOutcome::Canceled => Ok(None),
        kairo_control::SubmissionOutcome::Waiting { reason } => {
            if let Some(signal) = reason.strip_prefix("signal:") {
                status(
                    "36",
                    "●",
                    &format!("waiting for {signal} · worker released"),
                );
                println!("next · kairo signal {id}");
            } else {
                status("36", "●", "waiting for its timer · worker released");
                println!("next · kairo inspect {id}");
            }
            Ok(None)
        }
    }
}

// thin wrapper: the actual endpoint/worker-process lifecycle is shared with any other front end
// (e.g. the TUI) via `kairo_control::ensure_endpoint` -- this only adds the project-local worker
// default (a CLI/project-config concern), status printing, and this crate's own `CliError`.
pub(crate) fn ensure_endpoint(
    workers: Option<usize>,
    allow_console: bool,
    verbose: bool,
) -> Result<(kairo_control::Endpoint, Option<LocalService>)> {
    let default_workers = setup::project_workers()?.unwrap_or(DEFAULT_LOCAL_WORKERS);
    let (endpoint, local) = kairo_control::ensure_endpoint(
        Path::new(".kairo"),
        workers,
        default_workers,
        allow_console,
        verbose,
    )
    .map_err(|error| match error {
        kairo_control::ControlError::WorkersIgnored => CliError::WatchWorkers,
        kairo_control::ControlError::NoHealthyWorkers => CliError::NoWorkers,
        error => CliError::Control(error),
    })?;
    if let Some(local) = &local {
        let _ = local;
        status(
            "32",
            "✓",
            &format!(
                "local session ready · {} workers",
                workers.unwrap_or(default_workers)
            ),
        );
    }
    Ok((endpoint, local))
}

pub(crate) fn serve(workers: usize, allow_console: bool, verbose: bool) -> Result<()> {
    let server = kairo_control::Server::start(Path::new(".kairo"))?;
    let mut children = Vec::with_capacity(workers);
    for index in 1..=workers {
        children.push(kairo_control::start_worker(index, allow_console, verbose)?);
    }
    let result = server.serve().map_err(Into::into);
    kairo_control::stop_workers(&mut children);
    let _ = std::fs::remove_file(".kairo/control.json");
    let _ = std::fs::remove_file(".kairo/service.lock");
    result
}

/// the primary, intent-oriented health check -- no worker id, no group/control-plane detail.
/// `--verbose` (the CLI's existing global flag) reveals the exact same detail `kairo workers`
/// always shows.
pub(crate) fn print_status(verbose: bool) -> Result<()> {
    if verbose {
        return print_workers();
    }
    let endpoint = match kairo_control::load_endpoint(Path::new(".kairo")) {
        Ok(endpoint) => endpoint,
        Err(kairo_control::ControlError::Unavailable) => {
            println!("kairo · not running\nnext · kairo up");
            return Ok(());
        }
        Err(error) => return Err(error.into()),
    };
    let snapshot = match kairo_control::snapshot(&endpoint) {
        Ok(snapshot) => snapshot,
        Err(kairo_control::ControlError::Unavailable) => {
            println!("kairo · unavailable\nnext · kairo up");
            return Ok(());
        }
        Err(error) => return Err(error.into()),
    };
    let healthy = snapshot
        .workers
        .iter()
        .filter(|worker| worker.healthy)
        .count();
    if healthy == 0 {
        println!("kairo · not ready\nruntime · no healthy workers\nnext · kairo up");
        return Ok(());
    }
    println!("kairo · ready");
    println!("runtime · running");
    println!("scale · {healthy}");
    Ok(())
}

pub(crate) fn print_workers() -> Result<()> {
    let endpoint = match kairo_control::load_endpoint(Path::new(".kairo")) {
        Ok(endpoint) => endpoint,
        Err(kairo_control::ControlError::Unavailable) => {
            println!("no local service · run `kairo start`");
            return Ok(());
        }
        Err(error) => return Err(error.into()),
    };
    let snapshot = match kairo_control::snapshot(&endpoint) {
        Ok(snapshot) => snapshot,
        Err(kairo_control::ControlError::Unavailable) => {
            println!("local service is unavailable · run `kairo start`");
            return Ok(());
        }
        Err(error) => return Err(error.into()),
    };
    if snapshot.workers.is_empty() {
        println!("no workers connected");
        return Ok(());
    }
    println!("workers · {}", snapshot.workers.len());
    for worker in snapshot.workers {
        println!(
            "  {} · {} · {}",
            worker.id,
            if worker.healthy {
                "available"
            } else {
                "disconnected"
            },
            if worker.busy {
                "running a workflow"
            } else {
                "idle"
            },
        );
    }
    Ok(())
}
