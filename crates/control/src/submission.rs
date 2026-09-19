use std::{
    fs,
    path::{Path, PathBuf},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use kairo_core::{Workflow, WorkflowWait};
use kairo_storage::StorageConfig;
use sha2::{Digest, Sha256};

use crate::{
    ControlError, Endpoint, ReplayComponent, ReplayLineage, RunOutput, RunRequest, RunStatus,
    WaitRequest, client,
};

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
    submit_with_input(
        endpoint,
        workflow,
        workflow_path,
        state,
        storage,
        None,
        false,
    )
}

/// submits an ordinary stream workflow through the same durable worker lifecycle. The path is
/// intentionally only a local-input reference: cross-machine recovery must use an explicit
/// durable artifact, never an assumed shared filesystem.
pub fn submit_stream_run(
    endpoint: &Endpoint,
    workflow: &Workflow,
    workflow_path: &Path,
    state: &Path,
    storage: Option<StorageConfig>,
    input: Option<PathBuf>,
) -> Result<String, ControlError> {
    submit_with_input(
        endpoint,
        workflow,
        workflow_path,
        state,
        storage,
        input,
        true,
    )
}

fn submit_with_input(
    endpoint: &Endpoint,
    workflow: &Workflow,
    workflow_path: &Path,
    state: &Path,
    storage: Option<StorageConfig>,
    stream_input: Option<PathBuf>,
    stream_lineage: bool,
) -> Result<String, ControlError> {
    let workflow_path = absolute_path(workflow_path)?;
    let state = absolute_path(state)?;
    let mut storage = storage;
    if let Some(config) = storage.as_mut().filter(|config| config.local) {
        config.endpoint = absolute_path(Path::new(&config.endpoint))?
            .to_string_lossy()
            .into_owned();
    }
    let stream_input = stream_input.map(|path| absolute_path(&path)).transpose()?;
    let id = state.file_stem().map_or_else(
        || workflow.name().to_owned(),
        |name| name.to_string_lossy().into_owned(),
    );
    let lineage = stream_lineage
        .then(|| capture_lineage(&id, &workflow_path, workflow))
        .transpose()?;
    client::submit_with_lineage(
        endpoint,
        RunRequest {
            id: id.clone(),
            workflow: workflow_path,
            state,
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
            stream_input,
        },
        lineage,
    )?;
    Ok(id)
}

fn absolute_path(path: &Path) -> Result<PathBuf, ControlError> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()
            .map_err(|source| ControlError::ResolvePath { source })?
            .join(path))
    }
}

fn capture_lineage(
    run_id: &str,
    workflow_path: &Path,
    workflow: &Workflow,
) -> Result<ReplayLineage, ControlError> {
    let source = fs::read(workflow_path).map_err(|source| ControlError::Io { source })?;
    let components = workflow
        .steps()
        .iter()
        .map(|step| {
            let bytes = fs::read(&step.component).map_err(|source| ControlError::Io { source })?;
            Ok(ReplayComponent {
                step: step.id.to_string(),
                path: step.component.clone(),
                hash: format!("sha256:{:x}", Sha256::digest(bytes)),
            })
        })
        .collect::<Result<Vec<_>, ControlError>>()?;
    Ok(ReplayLineage {
        source_run: run_id.to_owned(),
        workflow_hash: format!("sha256:{:x}", Sha256::digest(source)),
        components,
        boundaries: Vec::new(),
    })
}

/// how a submitted run finished, or the reason it detached without finishing -- callers decide
/// what (if anything) to print; this module has no terminal/presentation concerns of its own.
pub enum SubmissionOutcome {
    Completed(RunOutput),
    /// the run paused for its wait boundary and released its worker; `reason` is e.g.
    /// `"signal:approval.granted"` or `"timer"`.
    Waiting {
        reason: String,
    },
    Canceled,
}

#[derive(Clone, Debug)]
pub struct ReplaySource {
    pub request: RunRequest,
    pub lineage: ReplayLineage,
}

pub fn replay_source(directory: &Path, id: &str) -> Result<ReplaySource, ControlError> {
    let persisted = crate::persistence::load(directory)?;
    let source = persisted.runs.get(id).ok_or(ControlError::State)?;
    if !matches!(source.status, RunStatus::Completed { .. }) {
        return Err(ControlError::Rejected {
            message: format!("run `{id}` is not completed"),
        });
    }
    let lineage = source
        .lineage
        .clone()
        .ok_or_else(|| ControlError::Rejected {
            message: format!("run `{id}` has no durable replay lineage"),
        })?;
    Ok(ReplaySource {
        request: source.request.clone(),
        lineage,
    })
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
