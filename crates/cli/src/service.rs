use std::{
    path::Path,
    process::{Child, Command as ProcessCommand, Stdio},
    thread,
    time::Duration,
};

use kairo_core::Workflow;

use crate::{CliError, Result, setup, status};

pub(crate) async fn submit_run(
    endpoint: &kairo_control::Endpoint,
    workflow: &Workflow,
    workflow_path: &Path,
    state: &Path,
) -> Result<()> {
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
        },
    )?;
    status("36", "→", &format!("queued {id}"));
    loop {
        match kairo_control::status(endpoint, id.clone())? {
            Some(kairo_control::RunStatus::Completed { output }) => {
                status("32", "✓", &format!("completed {}", workflow.name()));
                println!("{output}");
                return Ok(());
            }
            Some(kairo_control::RunStatus::Failed { message }) => {
                return Err(CliError::Control(kairo_control::ControlError::Rejected {
                    message,
                }));
            }
            Some(kairo_control::RunStatus::Queued | kairo_control::RunStatus::Running { .. }) => {
                thread::sleep(Duration::from_millis(50));
            }
            None => return Err(CliError::Control(kairo_control::ControlError::State)),
        }
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
