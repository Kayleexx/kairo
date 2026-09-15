use std::{
    path::Path,
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use kairo_core::{Workflow, WorkflowWait};
use kairo_storage::StorageConfig;

use crate::{ControlError, Endpoint, RunRequest, RunStatus, WaitRequest, client};

/// builds and submits a `RunRequest` for a workflow that needs the control plane (a durable,
/// waiting, or effect-carrying edge) -- shared by any front end (CLI, TUI) that has an already
/// established `Endpoint`. Never starts or manages a local service itself.
pub fn submit_run(
    endpoint: &Endpoint,
    workflow: &Workflow,
    workflow_path: &Path,
    state: &Path,
    storage: Option<StorageConfig>,
) -> Result<String, ControlError> {
    let id = state.file_stem().map_or_else(
        || workflow.name().to_owned(),
        |name| name.to_string_lossy().into_owned(),
    );
    client::submit(
        endpoint,
        RunRequest {
            id: id.clone(),
            workflow: workflow_path.to_path_buf(),
            state: state.to_path_buf(),
            storage,
            wait: workflow
                .wait()
                .filter(|_| workflow.wait_after().is_none())
                .map(wait_request)
                .transpose()?,
            plan: None,
            resume: None,
            preferred_worker: None,
            preferred_deadline_ms: None,
            shape: None,
        },
    )?;
    Ok(id)
}

/// how a submitted run finished, or the reason it detached without finishing -- callers decide
/// what (if anything) to print; this module has no terminal/presentation concerns of its own.
pub enum SubmissionOutcome {
    Completed(u32),
    /// the run paused for its wait boundary and released its worker; `reason` is e.g.
    /// `"signal:approval.granted"` or `"timer"`.
    Waiting {
        reason: String,
    },
    Canceled,
}

/// polls a submitted run to completion, or until it detaches for a wait boundary (when
/// `detach_on_wait`) or is canceled.
pub fn await_run(
    endpoint: &Endpoint,
    id: &str,
    detach_on_wait: bool,
) -> Result<SubmissionOutcome, ControlError> {
    loop {
        match client::status(endpoint, id.to_owned())? {
            Some(RunStatus::Completed { output, .. }) => {
                return Ok(SubmissionOutcome::Completed(output));
            }
            Some(RunStatus::Failed { message }) => return Err(ControlError::Rejected { message }),
            Some(RunStatus::CancelRequested { .. } | RunStatus::Canceled) => {
                return Ok(SubmissionOutcome::Canceled);
            }
            Some(RunStatus::Waiting { reason }) if detach_on_wait => {
                return Ok(SubmissionOutcome::Waiting { reason });
            }
            Some(RunStatus::Queued | RunStatus::Running { .. } | RunStatus::Waiting { .. }) => {
                thread::sleep(Duration::from_millis(50));
            }
            None => return Err(ControlError::State),
        }
    }
}

fn wait_request(wait: &WorkflowWait) -> Result<WaitRequest, ControlError> {
    match wait {
        WorkflowWait::Timer(duration) => {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|_| ControlError::State)?
                .as_millis();
            let due = now
                .saturating_add(duration.as_millis())
                .min(u128::from(u64::MAX)) as u64;
            Ok(WaitRequest::Timer { due_ms: due })
        }
        WorkflowWait::Signal(name) => Ok(WaitRequest::Signal { name: name.clone() }),
    }
}
