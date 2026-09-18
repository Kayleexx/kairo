use std::{
    path::Path,
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use kairo_core::Workflow;

use crate::{ControlError, Endpoint, Server};

pub struct LocalService {
    stopped: Arc<AtomicBool>,
    server: Option<thread::JoinHandle<Result<(), ControlError>>>,
    workers: Vec<Child>,
}

impl LocalService {
    pub fn stop(&mut self) -> Result<(), ControlError> {
        self.stopped.store(true, Ordering::Relaxed);
        stop_workers(&mut self.workers);
        let Some(server) = self.server.take() else {
            return Ok(());
        };
        let result = match server.join() {
            Ok(result) => result,
            Err(_) => Err(ControlError::ServiceThread),
        };
        let _ = std::fs::remove_file(".kairo/control.json");
        let _ = std::fs::remove_file(".kairo/service.lock");
        result
    }
}

impl Drop for LocalService {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

/// prefers an already-running service (so `kairo up` stays a sticky pool); otherwise starts an
/// ephemeral one the caller must `stop()`. `default_workers` is only used when a new service must
/// start and `workers` is `None`; the caller resolves any project-local default before calling, so
/// this module never needs to know about project configuration.
pub fn ensure_endpoint(
    directory: &Path,
    workers: Option<usize>,
    default_workers: usize,
    allow_console: bool,
    verbose: bool,
) -> Result<(Endpoint, Option<LocalService>), ControlError> {
    match crate::load_endpoint(directory) {
        Ok(endpoint) => match crate::snapshot(&endpoint) {
            Ok(snapshot) => {
                if workers.is_some() {
                    return Err(ControlError::WorkersIgnored);
                }
                if !snapshot.workers.iter().any(|worker| worker.healthy) {
                    return Err(ControlError::NoHealthyWorkers);
                }
                Ok((endpoint, None))
            }
            Err(ControlError::Unavailable) => start_local(
                directory,
                workers.unwrap_or(default_workers),
                allow_console,
                verbose,
            ),
            Err(error) => Err(error),
        },
        Err(ControlError::Unavailable) => start_local(
            directory,
            workers.unwrap_or(default_workers),
            allow_console,
            verbose,
        ),
        Err(error) => Err(error),
    }
}

fn start_local(
    directory: &Path,
    workers: usize,
    allow_console: bool,
    verbose: bool,
) -> Result<(Endpoint, Option<LocalService>), ControlError> {
    let server = Arc::new(Server::start(directory)?);
    let endpoint = server.endpoint().clone();
    let stopped = Arc::new(AtomicBool::new(false));
    let shutdown = Arc::clone(&stopped);
    let server = Arc::clone(&server);
    let handle = thread::spawn(move || server.serve_until(&shutdown));
    let mut local = LocalService {
        stopped,
        server: Some(handle),
        workers: Vec::with_capacity(workers),
    };
    for index in 1..=workers {
        local
            .workers
            .push(start_worker(index, allow_console, verbose)?);
    }
    if let Err(error) = wait_for_workers(&endpoint, workers, &mut local) {
        let _ = local.stop();
        return Err(error);
    }
    Ok((endpoint, Some(local)))
}

fn wait_for_workers(
    endpoint: &Endpoint,
    expected: usize,
    local: &mut LocalService,
) -> Result<(), ControlError> {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match crate::snapshot(endpoint) {
            Ok(snapshot)
                if snapshot
                    .workers
                    .iter()
                    .filter(|worker| worker.healthy)
                    .count()
                    >= expected =>
            {
                return Ok(());
            }
            Ok(_) | Err(ControlError::Unavailable) => {}
            Err(error) => return Err(error),
        }
        for child in &mut local.workers {
            if child
                .try_wait()
                .map_err(|source| ControlError::StartWorker { source })?
                .is_some()
            {
                return Err(ControlError::NoHealthyWorkers);
            }
        }
        if Instant::now() >= deadline {
            return Err(ControlError::NoHealthyWorkers);
        }
        thread::sleep(Duration::from_millis(20));
    }
}

/// spawns this same running binary re-invoked as `worker --id <id>` -- works from any process
/// that *is* the `kairo` binary, regardless of which subcommand launched it. `verbose` matters
/// here specifically: a worker is a separate process that never inherits the CLI flags of
/// whichever command started it, so without forwarding it explicitly, a durable/managed run's
/// actual component execution (which happens inside the worker, not the calling process) would
/// silently produce none of the diagnostics `--verbose` promised.
pub fn start_worker(
    index: usize,
    allow_console: bool,
    verbose: bool,
) -> Result<Child, ControlError> {
    let mut command = Command::new(
        std::env::current_exe().map_err(|source| ControlError::StartWorker { source })?,
    );
    command.args(["worker", "--id", &format!("worker-{index}")]);
    if allow_console {
        command.arg("--allow-console");
    }
    if verbose {
        command.arg("--verbose");
    }
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::inherit())
        .spawn()
        .map_err(|source| ControlError::StartWorker { source })
}

pub fn stop_workers(children: &mut [Child]) {
    for child in children {
        let _ = child.kill();
        let _ = child.wait();
    }
}

pub struct LocalEffect(Option<Child>);

impl Drop for LocalEffect {
    fn drop(&mut self) {
        if let Some(child) = &mut self.0 {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// starts this same running binary re-invoked as `effects serve`, if this workflow declares an
/// `effect:` boundary and no effect service is already reachable. A no-op otherwise.
pub fn ensure_effect_service(workflow: &Workflow) -> Result<LocalEffect, ControlError> {
    if workflow.effect().is_none() || effect_service_available() {
        return Ok(LocalEffect(None));
    }
    let mut child = Command::new(
        std::env::current_exe().map_err(|source| ControlError::StartWorker { source })?,
    )
    .args(["effects", "serve"])
    .stdin(Stdio::null())
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .spawn()
    .map_err(|source| ControlError::EffectService(source.to_string()))?;
    let deadline = Instant::now() + Duration::from_secs(2);
    while !effect_service_available() {
        if let Some(status) = child
            .try_wait()
            .map_err(|source| ControlError::EffectService(source.to_string()))?
        {
            let _ = child.wait();
            return Err(ControlError::EffectService(format!(
                "local effect service exited with {status}"
            )));
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(ControlError::EffectService(
                "local effect service did not become ready".to_owned(),
            ));
        }
        thread::sleep(Duration::from_millis(20));
    }
    Ok(LocalEffect(Some(child)))
}

fn effect_service_available() -> bool {
    std::fs::read_to_string(".kairo/effects.addr")
        .ok()
        .and_then(|value| value.trim().parse().ok())
        .is_some_and(|address| {
            std::net::TcpStream::connect_timeout(&address, Duration::from_millis(100)).is_ok()
        })
}
