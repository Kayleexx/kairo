use std::{
    fs::{self, OpenOptions},
    io::ErrorKind,
    path::Path,
    process::{Command as ProcessCommand, Stdio},
    thread,
    time::{Duration, Instant},
};

use crate::{CliError, Result, service, status};

const DIRECTORY: &str = ".kairo";
const LOCK: &str = ".kairo/service.lock";

pub(crate) fn start(workers: usize, foreground: bool) -> Result<()> {
    start_with_console(workers, foreground, false)
}

pub(crate) fn start_with_console(
    workers: usize,
    foreground: bool,
    allow_console: bool,
) -> Result<()> {
    if foreground {
        return service::serve(workers, allow_console);
    }
    if let Some(count) = connected_workers() {
        status("32", "✓", &format!("local service ready · {count} workers"));
        return Ok(());
    }
    if !acquire_lock()? {
        let count = connected_workers().unwrap_or_default();
        status("32", "✓", &format!("local service ready · {count} workers"));
        return Ok(());
    }
    let mut command = ProcessCommand::new(
        std::env::current_exe().map_err(|source| CliError::StartWorker { source })?,
    );
    command.args(["serve", "--workers", &workers.to_string()]);
    if allow_console {
        command.arg("--allow-console");
    }
    let child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    if let Err(source) = child {
        let _ = fs::remove_file(LOCK);
        return Err(CliError::StartWorker { source });
    }
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        if let Some(count) = connected_workers().filter(|count| *count >= workers) {
            status("32", "✓", &format!("local service ready · {count} workers"));
            return Ok(());
        }
        thread::sleep(Duration::from_millis(20));
    }
    let _ = fs::remove_file(LOCK);
    Err(CliError::Control(kairo_control::ControlError::Unavailable))
}

pub(crate) fn stop() -> Result<()> {
    let endpoint = match kairo_control::load_endpoint(Path::new(DIRECTORY)) {
        Ok(endpoint) => endpoint,
        Err(kairo_control::ControlError::Unavailable) => {
            remove_stale_files();
            status("32", "✓", "no local service is running");
            return Ok(());
        }
        Err(error) => return Err(error.into()),
    };
    match kairo_control::shutdown(&endpoint) {
        Ok(()) => {}
        Err(kairo_control::ControlError::Protocol { .. }) => return Err(CliError::ServiceUpgrade),
        Err(kairo_control::ControlError::Unavailable) => {
            remove_stale_files();
            status("32", "✓", "no local service is running");
            return Ok(());
        }
        Err(error) => return Err(error.into()),
    }
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        if connected_workers().is_none() {
            remove_stale_files();
            status("32", "✓", "local service stopped");
            return Ok(());
        }
        thread::sleep(Duration::from_millis(20));
    }
    Err(CliError::Control(kairo_control::ControlError::Unavailable))
}

fn connected_workers() -> Option<usize> {
    let endpoint = kairo_control::load_endpoint(Path::new(DIRECTORY)).ok()?;
    let snapshot = kairo_control::snapshot(&endpoint).ok()?;
    Some(
        snapshot
            .workers
            .iter()
            .filter(|worker| worker.healthy)
            .count(),
    )
}

fn acquire_lock() -> Result<bool> {
    fs::create_dir_all(DIRECTORY).map_err(|source| {
        CliError::Control(kairo_control::ControlError::CreateDirectory {
            path: Path::new(DIRECTORY).to_path_buf(),
            source,
        })
    })?;
    match OpenOptions::new().write(true).create_new(true).open(LOCK) {
        Ok(_) => Ok(true),
        Err(source) if source.kind() == ErrorKind::AlreadyExists => {
            let deadline = Instant::now() + Duration::from_secs(2);
            while Instant::now() < deadline {
                if connected_workers().is_some() {
                    return Ok(false);
                }
                thread::sleep(Duration::from_millis(20));
            }
            fs::remove_file(LOCK)
                .map_err(|source| CliError::Control(kairo_control::ControlError::Io { source }))?;
            acquire_lock()
        }
        Err(source) => Err(CliError::Control(kairo_control::ControlError::Io {
            source,
        })),
    }
}

fn remove_stale_files() {
    let _ = fs::remove_file(".kairo/control.json");
    let _ = fs::remove_file(LOCK);
}
