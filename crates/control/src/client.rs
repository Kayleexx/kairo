use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpStream,
    time::Duration,
};

use crate::{
    ControlError, Endpoint, ReplayLineage, Request, Response, RunPlan, RunRequest, RunStatus,
    Snapshot,
};

mod live_edge;
mod worker;

pub use live_edge::{
    begin_live_edge, complete_live_edge, complete_live_edge_output,
    complete_live_edge_with_metrics, fail_live_edge, live_edge, ready_live_edge,
    streaming_live_edge,
};
pub use worker::{worker_loop, worker_loop_with_assignments, worker_loop_with_waits};

const MAX_MESSAGE_BYTES: u64 = 1024 * 1024;

/// `Canceled` is an expected, non-fatal outcome, distinct from a real rejection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ReportOutcome {
    Accepted,
    Canceled,
}

fn report_outcome(response: Response) -> Result<ReportOutcome, ControlError> {
    match response {
        Response::Ok => Ok(ReportOutcome::Accepted),
        Response::Canceled => Ok(ReportOutcome::Canceled),
        Response::Error { message } => Err(ControlError::Rejected { message }),
        _ => Err(ControlError::State),
    }
}

pub fn submit(endpoint: &Endpoint, run: RunRequest) -> Result<(), ControlError> {
    submit_with_lineage(endpoint, run, None)
}

pub fn submit_with_lineage(
    endpoint: &Endpoint,
    run: RunRequest,
    lineage: Option<ReplayLineage>,
) -> Result<(), ControlError> {
    ok(request(
        endpoint,
        Request::Submit {
            token: endpoint.token.clone(),
            run,
            lineage: lineage.map(Box::new),
        },
    )?)
}

pub fn cancel(endpoint: &Endpoint, id: String) -> Result<(), ControlError> {
    ok(request(
        endpoint,
        Request::Cancel {
            token: endpoint.token.clone(),
            id,
        },
    )?)
}

/// erases a finished run's record from the control service entirely -- for internal, short-lived
/// runs (like `kairo bench --profile`'s measurement passes) that should never linger as ordinary
/// user-visible history. Refused for a run not yet in a terminal state.
pub fn forget(endpoint: &Endpoint, id: String) -> Result<(), ControlError> {
    ok(request(
        endpoint,
        Request::Forget {
            token: endpoint.token.clone(),
            id,
        },
    )?)
}

pub fn status(endpoint: &Endpoint, id: String) -> Result<Option<RunStatus>, ControlError> {
    match request(
        endpoint,
        Request::Status {
            token: endpoint.token.clone(),
            id,
        },
    )? {
        Response::Status { status } => Ok(status),
        Response::Error { message } => Err(ControlError::Rejected { message }),
        _ => Err(ControlError::State),
    }
}

pub fn snapshot(endpoint: &Endpoint) -> Result<Snapshot, ControlError> {
    match request(
        endpoint,
        Request::Snapshot {
            token: endpoint.token.clone(),
        },
    )? {
        Response::Snapshot { snapshot } => Ok(snapshot),
        Response::Error { message } => Err(ControlError::Rejected { message }),
        _ => Err(ControlError::State),
    }
}

pub fn kill_worker(endpoint: &Endpoint, worker: String) -> Result<(), ControlError> {
    ok(request(
        endpoint,
        Request::ChaosKill {
            token: endpoint.token.clone(),
            worker,
        },
    )?)
}

pub fn signal(endpoint: &Endpoint, id: String, signal: String) -> Result<(), ControlError> {
    ok(request(
        endpoint,
        Request::Signal {
            token: endpoint.token.clone(),
            id,
            signal,
        },
    )?)
}

/// reports that a worker reached an ExecutionGroup boundary and is handing the run's remainder
/// back to the queue -- `plan` is `Some` only on a run's first yield (subsequent groups reuse the
/// already-persisted plan), and `target_worker`/`target_had_cache` record gate 3's placement
/// choice, if any.
#[allow(clippy::too_many_arguments)]
pub fn yield_group(
    endpoint: &Endpoint,
    worker: &str,
    id: String,
    epoch: u64,
    next_index: usize,
    artifact_hash: String,
    artifact_backend: String,
    plan: Option<RunPlan>,
    shape: Option<String>,
    target_worker: Option<String>,
    target_had_cache: bool,
) -> Result<(), ControlError> {
    ok(request(
        endpoint,
        Request::Yield {
            worker: worker.to_owned(),
            token: endpoint.token.clone(),
            id,
            epoch,
            next_index,
            artifact_hash,
            artifact_backend,
            plan,
            shape,
            target_worker,
            target_had_cache,
        },
    )?)
}

pub fn shutdown(endpoint: &Endpoint) -> Result<(), ControlError> {
    ok(request(
        endpoint,
        Request::Shutdown {
            token: endpoint.token.clone(),
        },
    )?)
}

fn request(endpoint: &Endpoint, request: Request) -> Result<Response, ControlError> {
    let mut stream = TcpStream::connect_timeout(&endpoint.address, Duration::from_millis(500))
        .map_err(|source| {
            if matches!(
                source.kind(),
                std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::TimedOut
            ) {
                ControlError::Unavailable
            } else {
                ControlError::Io { source }
            }
        })?;
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .map_err(|source| ControlError::Io { source })?;
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .map_err(|source| ControlError::Io { source })?;
    let bytes = serde_json::to_vec(&request).map_err(|source| ControlError::Protocol { source })?;
    if bytes.len() as u64 > MAX_MESSAGE_BYTES {
        return Err(ControlError::TooLarge);
    }
    stream
        .write_all(&bytes)
        .map_err(|source| ControlError::Io { source })?;
    stream
        .write_all(b"\n")
        .map_err(|source| ControlError::Io { source })?;
    read_line(&mut stream)
}

fn ok(response: Response) -> Result<(), ControlError> {
    match response {
        Response::Ok => Ok(()),
        Response::Error { message } => Err(ControlError::Rejected { message }),
        _ => Err(ControlError::State),
    }
}

fn read_line<T: for<'a> serde::Deserialize<'a>>(stream: &mut TcpStream) -> Result<T, ControlError> {
    let mut bytes = Vec::new();
    BufReader::new(stream)
        .take(MAX_MESSAGE_BYTES.saturating_add(1))
        .read_until(b'\n', &mut bytes)
        .map_err(|source| ControlError::Io { source })?;
    if bytes.len() as u64 > MAX_MESSAGE_BYTES {
        return Err(ControlError::TooLarge);
    }
    serde_json::from_slice(&bytes).map_err(|source| ControlError::Protocol { source })
}
