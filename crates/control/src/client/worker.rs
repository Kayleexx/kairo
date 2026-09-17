use std::{
    sync::{Arc, mpsc},
    thread,
    time::Duration,
};

use crate::{
    Assignment, ControlError, Endpoint, LiveEdgeParticipant, Request, Response, RunOutput,
    RunRequest, RunStatus, WaitRequest, WorkerResult,
};

use super::{
    ReportOutcome, complete_live_edge_output, complete_live_edge_with_metrics, fail_live_edge,
    live_edge, report_outcome, request, status, yield_group,
};

pub fn worker_loop(
    endpoint: Endpoint,
    worker: String,
    execute: impl Fn(RunRequest) -> Result<u32, String> + Send + Sync + 'static,
) -> Result<(), ControlError> {
    worker_loop_with_waits(endpoint, worker, move |run| {
        execute(run)
            .map(RunOutput::Scalar)
            .map(WorkerResult::Completed)
    })
}

pub fn worker_loop_with_waits(
    endpoint: Endpoint,
    worker: String,
    execute: impl Fn(RunRequest) -> Result<WorkerResult, String> + Send + Sync + 'static,
) -> Result<(), ControlError> {
    worker_loop_with_assignments(endpoint, worker, move |assignment| execute(assignment.run))
}

pub fn worker_loop_with_assignments(
    endpoint: Endpoint,
    worker: String,
    execute: impl Fn(Assignment) -> Result<WorkerResult, String> + Send + Sync + 'static,
) -> Result<(), ControlError> {
    register(&endpoint, &worker)?;
    let execute = Arc::new(execute);
    loop {
        heartbeat(&endpoint, &worker)?;
        let Some(assignment) = next(&endpoint, &worker)? else {
            thread::sleep(Duration::from_millis(100));
            continue;
        };
        if assignment.live_edge.is_some() {
            execute_live(&endpoint, &worker, &execute, assignment)?;
            continue;
        }
        execute_parent(&endpoint, &worker, &execute, assignment)?;
    }
}

fn execute_live(
    endpoint: &Endpoint,
    worker: &str,
    execute: &Arc<impl Fn(Assignment) -> Result<WorkerResult, String> + Send + Sync + 'static>,
    assignment: Assignment,
) -> Result<(), ControlError> {
    let (sender, receiver) = mpsc::sync_channel(1);
    let task = Arc::clone(execute);
    let task_assignment = assignment.clone();
    thread::spawn(move || {
        let _ = sender.send(task(task_assignment));
    });
    let result = loop {
        match receiver.recv_timeout(Duration::from_secs(1)) {
            Ok(result) => break result,
            Err(mpsc::RecvTimeoutError::Timeout) => heartbeat(endpoint, worker)?,
            Err(mpsc::RecvTimeoutError::Disconnected) => return Err(ControlError::State),
        }
    };
    let reported = match result {
        Ok(WorkerResult::Completed(output)) => complete_live_edge_output(
            endpoint,
            worker,
            &assignment,
            LiveEdgeParticipant::Consumer,
            Some(output),
        ),
        Ok(WorkerResult::LiveCompleted { output, metrics }) => complete_live_edge_with_metrics(
            endpoint,
            worker,
            &assignment,
            LiveEdgeParticipant::Consumer,
            Some(output),
            Some(metrics),
        ),
        Ok(_) => fail_live_edge(
            endpoint,
            worker,
            &assignment,
            LiveEdgeParticipant::Consumer,
            "live consumer did not produce a final result".to_owned(),
        ),
        Err(message) => fail_live_edge(
            endpoint,
            worker,
            &assignment,
            LiveEdgeParticipant::Consumer,
            message,
        ),
    };
    if let Err(error) = reported
        && !live_report_is_stale_terminal(endpoint, &assignment)?
    {
        return Err(error);
    }
    Ok(())
}

fn execute_parent(
    endpoint: &Endpoint,
    worker: &str,
    execute: &Arc<impl Fn(Assignment) -> Result<WorkerResult, String> + Send + Sync + 'static>,
    assignment: Assignment,
) -> Result<(), ControlError> {
    let id = assignment.run.id.clone();
    let epoch = assignment.epoch;
    let (sender, receiver) = mpsc::sync_channel(1);
    let task = Arc::clone(execute);
    thread::spawn(move || {
        let _ = sender.send(task(assignment));
    });
    loop {
        match receiver.recv_timeout(Duration::from_secs(1)) {
            Ok(Ok(WorkerResult::Completed(output))) => {
                let _outcome = complete(endpoint, worker, id.clone(), epoch, output)?;
                return Ok(());
            }
            Ok(Ok(WorkerResult::LiveCompleted { .. })) => return Err(ControlError::State),
            Ok(Ok(WorkerResult::Waiting(wait_request))) => {
                let _outcome = wait(endpoint, worker, id.clone(), epoch, wait_request)?;
                return Ok(());
            }
            Ok(Ok(WorkerResult::Yielded {
                next_index,
                artifact_hash,
                artifact_backend,
                plan,
                shape,
                target_worker,
                target_had_cache,
            })) => {
                yield_group(
                    endpoint,
                    worker,
                    id.clone(),
                    epoch,
                    next_index,
                    artifact_hash,
                    artifact_backend,
                    plan,
                    shape,
                    target_worker,
                    target_had_cache,
                )?;
                return Ok(());
            }
            Ok(Err(message)) => {
                if let Err(error) = fail(endpoint, worker, id.clone(), epoch, message)
                    && !run_report_is_stale(endpoint, &id, worker, epoch)?
                {
                    return Err(error);
                }
                return Ok(());
            }
            Err(mpsc::RecvTimeoutError::Timeout) => heartbeat(endpoint, worker)?,
            Err(mpsc::RecvTimeoutError::Disconnected) => return Err(ControlError::State),
        }
    }
}

fn run_report_is_stale(
    endpoint: &Endpoint,
    id: &str,
    worker: &str,
    epoch: u64,
) -> Result<bool, ControlError> {
    Ok(!matches!(
        status(endpoint, id.to_owned())?,
        Some(RunStatus::Running { worker: owner, epoch: assigned })
            if owner == worker && assigned == epoch
    ))
}

fn live_report_is_stale_terminal(
    endpoint: &Endpoint,
    assignment: &Assignment,
) -> Result<bool, ControlError> {
    let session_id = assignment
        .live_edge
        .as_ref()
        .ok_or(ControlError::State)?
        .session_id
        .clone();
    Ok(matches!(
        live_edge(endpoint, session_id)?.map(|session| session.state),
        Some(crate::LiveEdgeState::Completed)
            | Some(crate::LiveEdgeState::Failed { .. })
            | Some(crate::LiveEdgeState::Cancelled)
    ))
}

fn wait(
    endpoint: &Endpoint,
    worker: &str,
    id: String,
    epoch: u64,
    wait: WaitRequest,
) -> Result<ReportOutcome, ControlError> {
    report_outcome(request(
        endpoint,
        Request::Wait {
            worker: worker.to_owned(),
            token: endpoint.token.clone(),
            id,
            epoch,
            wait,
        },
    )?)
}

fn register(endpoint: &Endpoint, worker: &str) -> Result<(), ControlError> {
    super::ok(request(
        endpoint,
        Request::Register {
            worker: worker.to_owned(),
            pid: std::process::id(),
            token: endpoint.token.clone(),
        },
    )?)
}

fn heartbeat(endpoint: &Endpoint, worker: &str) -> Result<(), ControlError> {
    super::ok(request(
        endpoint,
        Request::Heartbeat {
            worker: worker.to_owned(),
            token: endpoint.token.clone(),
        },
    )?)
}

fn next(endpoint: &Endpoint, worker: &str) -> Result<Option<Assignment>, ControlError> {
    match request(
        endpoint,
        Request::Next {
            worker: worker.to_owned(),
            token: endpoint.token.clone(),
        },
    )? {
        Response::Assignment { run } => Ok(run.map(|boxed| *boxed)),
        Response::Error { message } => Err(ControlError::Rejected { message }),
        _ => Err(ControlError::State),
    }
}

fn complete(
    endpoint: &Endpoint,
    worker: &str,
    id: String,
    epoch: u64,
    output: RunOutput,
) -> Result<ReportOutcome, ControlError> {
    report_outcome(request(
        endpoint,
        Request::Complete {
            worker: worker.to_owned(),
            token: endpoint.token.clone(),
            id,
            epoch,
            output,
        },
    )?)
}

fn fail(
    endpoint: &Endpoint,
    worker: &str,
    id: String,
    epoch: u64,
    message: String,
) -> Result<ReportOutcome, ControlError> {
    report_outcome(request(
        endpoint,
        Request::Fail {
            worker: worker.to_owned(),
            token: endpoint.token.clone(),
            id,
            epoch,
            message,
        },
    )?)
}
