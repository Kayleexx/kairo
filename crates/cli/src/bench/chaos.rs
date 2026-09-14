use std::{
    path::Path,
    thread,
    time::{Duration, Instant},
};

use kairo_control::{Endpoint, RunEvent, RunStatus};
use kairo_core::Workflow;

use super::{report::RecoveryRecord, runner::BenchError};

const POLL_INTERVAL: Duration = Duration::from_millis(5);
// not a guessed completion time -- the lease timeout alone is a fixed 3s, so this just bounds a
// genuinely stuck poll loop (a real bug) rather than predicting how long recovery takes.
const POLL_DEADLINE: Duration = Duration::from_secs(30);

/// submits a real run, kills the real worker that picks it up, and polls (never sleeps for a
/// guessed duration) until it recovers on a survivor. Scalar only -- streams bypass the control
/// plane (see the Phase 13 design notes).
pub(crate) fn run_worker_kill_cycle(
    workflow_path: &Path,
    workflow: &Workflow,
    attempt: u32,
) -> Result<RecoveryRecord, BenchError> {
    // fresh service per cycle: a kill leaves one fewer worker, so a shared pool would run dry.
    // console output is allowed so the workflow stays observably busy long enough to be killed
    // mid-flight (see the Phase 13 design notes).
    let _ = crate::lifecycle::stop();
    crate::lifecycle::start_with_console(2, false, true)?;
    let endpoint = kairo_control::load_endpoint(Path::new(".kairo"))?;

    let state_path = crate::state::generated_run(workflow.name())?;
    let id = crate::service::submit(&endpoint, workflow, workflow_path, &state_path)?;

    let record = match poll_until_running(&endpoint, &id)? {
        None => RecoveryRecord {
            attempt,
            killed_worker: None,
            recovery_wall_ms: None,
            outcome: "too-fast-to-observe".to_owned(),
            reassignment_reason: None,
        },
        Some(worker) => {
            kairo_control::kill_worker(&endpoint, worker.clone())?;
            let killed_at = Instant::now();
            let outcome = poll_until_terminal(&endpoint, &id)?;
            let recovery_wall_ms = killed_at.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
            let reassignment_reason = reassignment_reason(&endpoint, &id);
            RecoveryRecord {
                attempt,
                killed_worker: Some(worker),
                recovery_wall_ms: Some(recovery_wall_ms),
                outcome,
                reassignment_reason,
            }
        }
    };

    cleanup(&state_path);
    let _ = crate::lifecycle::stop();
    Ok(record)
}

fn poll_until_running(endpoint: &Endpoint, id: &str) -> Result<Option<String>, BenchError> {
    let deadline = Instant::now() + POLL_DEADLINE;
    while Instant::now() < deadline {
        match kairo_control::status(endpoint, id.to_owned())? {
            Some(RunStatus::Running { worker, .. }) => return Ok(Some(worker)),
            Some(RunStatus::Completed { .. } | RunStatus::Failed { .. }) => return Ok(None),
            _ => thread::sleep(POLL_INTERVAL),
        }
    }
    Err(BenchError::ChaosTimeout { id: id.to_owned() })
}

fn poll_until_terminal(endpoint: &Endpoint, id: &str) -> Result<String, BenchError> {
    let deadline = Instant::now() + POLL_DEADLINE;
    while Instant::now() < deadline {
        match kairo_control::status(endpoint, id.to_owned())? {
            Some(RunStatus::Completed { .. }) => return Ok("completed".to_owned()),
            Some(RunStatus::Failed { .. }) => return Ok("failed".to_owned()),
            Some(RunStatus::Canceled) => return Ok("canceled".to_owned()),
            _ => thread::sleep(POLL_INTERVAL),
        }
    }
    Err(BenchError::ChaosTimeout { id: id.to_owned() })
}

/// the most recent assignment reason from the control plane's own history (Slice 13.1) -- the
/// first is always `Initial`, so only the last one reflects a real post-kill reassignment.
fn reassignment_reason(endpoint: &Endpoint, id: &str) -> Option<String> {
    let snapshot = kairo_control::snapshot(endpoint).ok()?;
    let history = snapshot.runs.into_iter().find(|run| run.id == id)?.history;
    history.into_iter().rev().find_map(|event| match event {
        RunEvent::Assigned { reason, .. } => Some(format!("{reason:?}")),
        _ => None,
    })
}

fn cleanup(path: &Path) {
    for extension in ["db", "db-shm", "db-wal", "lock"] {
        let _ = std::fs::remove_file(path.with_extension(extension));
    }
}
