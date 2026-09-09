use std::{
    path::Path,
    process::{Child, Command as ProcessCommand, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use kairo_core::{Workflow, WorkflowWait};

use crate::{CliError, Result, setup, status};

const WATCH_WORKERS: usize = 2;

struct LocalService {
    stopped: Arc<AtomicBool>,
    server: Option<thread::JoinHandle<Result<()>>>,
    workers: Vec<Child>,
}

pub(crate) fn submit_run(
    endpoint: &kairo_control::Endpoint,
    workflow: &Workflow,
    workflow_path: &Path,
    state: &Path,
) -> Result<()> {
    let id = submit(endpoint, workflow, workflow_path, state)?;
    status("36", "→", &format!("queued {id}"));
    let output = wait_for_output(endpoint, &id)?;
    status("32", "✓", &format!("completed {}", workflow.name()));
    println!("{output}");
    Ok(())
}

pub(crate) fn watch_run(
    workers: Option<usize>,
    workflow: &Workflow,
    workflow_path: &Path,
    state: Option<&Path>,
) -> Result<()> {
    let state = state.ok_or(kairo_control::ControlError::State)?;
    let (endpoint, mut local) = watch_endpoint(workers)?;
    let id = submit(&endpoint, workflow, workflow_path, state)?;
    status("36", "→", &format!("queued {id} · opening live activity"));
    kairo_tui::run()?;
    let output = wait_for_output(&endpoint, &id)?;
    if let Some(local) = &mut local {
        local.stop()?;
    }
    status("32", "✓", &format!("completed {}", workflow.name()));
    println!("{output}");
    Ok(())
}

fn submit(
    endpoint: &kairo_control::Endpoint,
    workflow: &Workflow,
    workflow_path: &Path,
    state: &Path,
) -> Result<String> {
    let id = state.file_stem().map_or_else(
        || workflow.name().to_owned(),
        |name| name.to_string_lossy().into_owned(),
    );
    let storage = workflow
        .requires_durable_artifacts()
        .then(setup::storage_config)
        .transpose()?;
    kairo_control::submit(
        endpoint,
        kairo_control::RunRequest {
            id: id.clone(),
            workflow: workflow_path.to_path_buf(),
            state: state.to_path_buf(),
            storage,
            wait: workflow.wait().map(wait_request).transpose()?,
        },
    )?;
    Ok(id)
}

fn wait_request(wait: &WorkflowWait) -> Result<kairo_control::WaitRequest> {
    match wait {
        WorkflowWait::Timer(duration) => {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|_| CliError::Control(kairo_control::ControlError::State))?
                .as_millis();
            let due = now
                .saturating_add(duration.as_millis())
                .min(u128::from(u64::MAX)) as u64;
            Ok(kairo_control::WaitRequest::Timer { due_ms: due })
        }
        WorkflowWait::Signal(name) => Ok(kairo_control::WaitRequest::Signal { name: name.clone() }),
    }
}

fn wait_for_output(endpoint: &kairo_control::Endpoint, id: &str) -> Result<u32> {
    loop {
        match kairo_control::status(endpoint, id.to_owned())? {
            Some(kairo_control::RunStatus::Completed { output, .. }) => return Ok(output),
            Some(kairo_control::RunStatus::Failed { message }) => {
                return Err(CliError::Control(kairo_control::ControlError::Rejected {
                    message,
                }));
            }
            Some(
                kairo_control::RunStatus::Queued
                | kairo_control::RunStatus::Running { .. }
                | kairo_control::RunStatus::Waiting { .. },
            ) => {
                thread::sleep(Duration::from_millis(50));
            }
            None => return Err(CliError::Control(kairo_control::ControlError::State)),
        }
    }
}

fn watch_endpoint(
    workers: Option<usize>,
) -> Result<(kairo_control::Endpoint, Option<LocalService>)> {
    match kairo_control::load_endpoint(Path::new(".kairo")) {
        Ok(endpoint) => match kairo_control::snapshot(&endpoint) {
            Ok(snapshot) => {
                if workers.is_some() {
                    return Err(CliError::WatchWorkers);
                }
                if !snapshot.workers.iter().any(|worker| worker.healthy) {
                    return Err(CliError::NoWorkers);
                }
                Ok((endpoint, None))
            }
            Err(kairo_control::ControlError::Unavailable) => {
                start_local(workers.unwrap_or(WATCH_WORKERS))
            }
            Err(error) => Err(error.into()),
        },
        Err(kairo_control::ControlError::Unavailable) => {
            start_local(workers.unwrap_or(WATCH_WORKERS))
        }
        Err(error) => Err(error.into()),
    }
}

fn start_local(workers: usize) -> Result<(kairo_control::Endpoint, Option<LocalService>)> {
    let server = Arc::new(kairo_control::Server::start(Path::new(".kairo"))?);
    let endpoint = server.endpoint().clone();
    let stopped = Arc::new(AtomicBool::new(false));
    let shutdown = Arc::clone(&stopped);
    let server = Arc::clone(&server);
    let handle = thread::spawn(move || server.serve_until(&shutdown).map_err(Into::into));
    let mut local = LocalService {
        stopped,
        server: Some(handle),
        workers: Vec::with_capacity(workers),
    };
    for index in 1..=workers {
        local.workers.push(start_worker(index)?);
    }
    status(
        "32",
        "✓",
        &format!("local session ready · {workers} workers"),
    );
    Ok((endpoint, Some(local)))
}

impl LocalService {
    fn stop(&mut self) -> Result<()> {
        self.stopped.store(true, Ordering::Relaxed);
        stop_workers(&mut self.workers);
        let Some(server) = self.server.take() else {
            return Ok(());
        };
        match server.join() {
            Ok(result) => result,
            Err(_) => Err(CliError::ServiceThread),
        }
    }
}

impl Drop for LocalService {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

pub(crate) fn start(workers: usize) -> Result<()> {
    let server = kairo_control::Server::start(Path::new(".kairo"))?;
    let mut children = Vec::with_capacity(workers);
    for index in 1..=workers {
        children.push(start_worker(index)?);
    }
    status(
        "32",
        "✓",
        &format!("local service ready · {workers} workers"),
    );
    let result = server.serve().map_err(Into::into);
    stop_workers(&mut children);
    result
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

pub(crate) fn signal(run: &str, signal: &str) -> Result<()> {
    if signal.is_empty() || signal.len() > 128 || signal.chars().any(char::is_control) {
        return Err(CliError::Control(kairo_control::ControlError::Rejected {
            message: "signal must be 1–128 printable characters".to_owned(),
        }));
    }
    let endpoint = kairo_control::load_endpoint(Path::new(".kairo"))?;
    kairo_control::signal(&endpoint, run.to_owned(), signal.to_owned())?;
    println!("signal accepted · {signal}");
    Ok(())
}

fn start_worker(index: usize) -> Result<Child> {
    ProcessCommand::new(std::env::current_exe().map_err(|source| CliError::StartWorker { source })?)
        .args(["worker", "--id", &format!("worker-{index}")])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|source| CliError::StartWorker { source })
}

fn stop_workers(children: &mut [Child]) {
    for child in children {
        let _ = child.kill();
        let _ = child.wait();
    }
}
