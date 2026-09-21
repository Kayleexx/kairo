use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use kairo_core::{Config, Workflow, WorkflowMode, WorkflowWait};
use kairo_storage::StorageConfig;
use sha2::{Digest, Sha256};

use crate::{
    ControlError, Endpoint, GroupResume, ReplayComponent, ReplayLineage, RunOutput, RunPlan,
    RunRequest, RunStatus, WaitRequest, client,
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
        .then(|| capture_lineage(&id, &workflow_path, workflow, stream_input.as_deref()))
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
    stream_input: Option<&Path>,
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
        original_run: run_id.to_owned(),
        workflow_hash: format!("sha256:{:x}", Sha256::digest(source)),
        components,
        input_hash: stream_input
            .or_else(|| workflow.stream_input())
            .map(hash_file)
            .transpose()?,
        resolved_durability: Default::default(),
        boundaries: Vec::new(),
    })
}

fn hash_file(path: &Path) -> Result<String, ControlError> {
    let mut file = fs::File::open(path).map_err(|source| ControlError::Io { source })?;
    let mut hash = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|source| ControlError::Io { source })?;
        if read == 0 {
            return Ok(format!("sha256:{:x}", hash.finalize()));
        }
        hash.update(&buffer[..read]);
    }
}

/// validates immutable source lineage and constructs a new child request. artifact availability
/// is checked by the caller before submission because object stores are asynchronous.
pub fn prepare_replay(
    directory: &Path,
    source_id: &str,
    until: &str,
    child_state: PathBuf,
    boundary: Option<GroupResume>,
) -> Result<(RunRequest, ReplayLineage), ControlError> {
    let source = replay_source(directory, source_id)?;
    let workflow = Workflow::load(
        &source.request.workflow,
        Config::default().max_workflow_bytes,
        Config::default().max_workflow_steps,
    )
    .map_err(|error| ControlError::Rejected {
        message: format!("source workflow is no longer valid: {error}"),
    })?;
    if workflow.mode() != WorkflowMode::Stream {
        return Err(reject("durable replay currently supports stream workflows"));
    }
    if workflow.effect().is_some() || workflow.wait().is_some() {
        return Err(reject(
            "replay refuses workflows with effects or waits without a proven reusable receipt",
        ));
    }
    validate_lineage(&source.lineage, &source.request.workflow, &workflow)?;
    let until_index = workflow
        .steps()
        .iter()
        .position(|step| step.id.as_str() == until)
        .ok_or_else(|| reject(format!("step `{until}` is not in the source workflow")))?;
    if let Some(boundary) = &boundary {
        if boundary.from_index > until_index
            || !source.lineage.boundaries.iter().any(|known| {
                known.from_index == boundary.from_index
                    && known.artifact_hash == boundary.artifact_hash
                    && known.artifact_backend == boundary.artifact_backend
            })
        {
            return Err(reject(
                "selected replay boundary is not valid for the requested step",
            ));
        }
    } else {
        validate_input(&source.lineage, &source.request, &workflow)?;
    }
    let id = child_state
        .file_stem()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .ok_or_else(|| reject("replay child state needs a file name"))?;
    let mut plan = RunPlan {
        resolved_durability: source.lineage.resolved_durability.clone(),
        replay_until: Some(until_index),
        replay_source: Some(source_id.to_owned()),
    };
    if plan.resolved_durability.is_empty() {
        plan.resolved_durability = source
            .request
            .plan
            .as_ref()
            .map(|plan| plan.resolved_durability.clone())
            .unwrap_or_default();
    }
    let request = RunRequest {
        id,
        workflow: source.request.workflow,
        state: child_state,
        storage: source.request.storage,
        wait: None,
        plan: Some(plan),
        resume: boundary,
        preferred_worker: None,
        preferred_deadline_ms: None,
        shape: source.request.shape,
        stream_input: source.request.stream_input,
    };
    let mut lineage = source.lineage;
    lineage.source_run = source_id.to_owned();
    if lineage.original_run.is_empty() {
        lineage.original_run = source_id.to_owned();
    }
    Ok((request, lineage))
}

fn validate_lineage(
    lineage: &ReplayLineage,
    workflow_path: &Path,
    workflow: &Workflow,
) -> Result<(), ControlError> {
    let source = fs::read(workflow_path).map_err(|source| ControlError::Io { source })?;
    if lineage.workflow_hash != format!("sha256:{:x}", Sha256::digest(source)) {
        return Err(reject(
            "source workflow hash no longer matches its durable lineage",
        ));
    }
    if lineage.components.len() != workflow.steps().len() {
        return Err(reject(
            "source component lineage does not match the workflow",
        ));
    }
    for (step, component) in workflow.steps().iter().zip(&lineage.components) {
        let bytes = fs::read(&step.component).map_err(|source| ControlError::Io { source })?;
        // identity is step id + content hash, not `component.path`'s spelling -- that path was
        // captured relative to the original submission's cwd, which reload can't reproduce.
        if component.step != step.id.as_str()
            || component.hash != format!("sha256:{:x}", Sha256::digest(bytes))
        {
            return Err(reject(
                "source component hash no longer matches its durable lineage",
            ));
        }
    }
    Ok(())
}

fn validate_input(
    lineage: &ReplayLineage,
    request: &RunRequest,
    workflow: &Workflow,
) -> Result<(), ControlError> {
    let expected = lineage
        .input_hash
        .as_ref()
        .ok_or_else(|| reject("source lineage has no input hash and no durable boundary"))?;
    let input = request
        .stream_input
        .as_deref()
        .or_else(|| workflow.stream_input())
        .ok_or_else(|| {
            reject("source input is unavailable and no durable boundary can be reused")
        })?;
    if hash_file(input)? != *expected {
        return Err(reject(
            "source input hash no longer matches its durable lineage",
        ));
    }
    Ok(())
}

fn reject(message: impl Into<String>) -> ControlError {
    ControlError::Rejected {
        message: message.into(),
    }
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
    let source = persisted
        .runs
        .get(id)
        .ok_or_else(|| ControlError::RunNotFound { id: id.to_owned() })?;
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
    // this is user-visible completion latency for every managed run. the control protocol is
    // request/response today, so keep the interval short without changing ownership or fencing.
    const COMPLETION_POLL_INTERVAL: Duration = Duration::from_millis(5);
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
                thread::sleep(COMPLETION_POLL_INTERVAL);
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
