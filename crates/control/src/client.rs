use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpStream,
    sync::{Arc, mpsc},
    thread,
    time::Duration,
};

use crate::{
    Assignment, ControlError, Endpoint, Request, Response, RunRequest, RunStatus, Snapshot,
    WaitRequest, WorkerResult,
};

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
    ok(request(
        endpoint,
        Request::Submit {
            token: endpoint.token.clone(),
            run,
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

pub fn shutdown(endpoint: &Endpoint) -> Result<(), ControlError> {
    ok(request(
        endpoint,
        Request::Shutdown {
            token: endpoint.token.clone(),
        },
    )?)
}

pub fn worker_loop(
    endpoint: Endpoint,
    worker: String,
    execute: impl Fn(RunRequest) -> Result<u32, String> + Send + Sync + 'static,
) -> Result<(), ControlError> {
    worker_loop_with_waits(endpoint, worker, move |run| {
        execute(run).map(WorkerResult::Completed)
    })
}

pub fn worker_loop_with_waits(
    endpoint: Endpoint,
    worker: String,
    execute: impl Fn(RunRequest) -> Result<WorkerResult, String> + Send + Sync + 'static,
) -> Result<(), ControlError> {
    register(&endpoint, &worker)?;
    let execute = Arc::new(execute);
    loop {
        heartbeat(&endpoint, &worker)?;
        let Some(assignment) = next(&endpoint, &worker)? else {
            thread::sleep(Duration::from_millis(100));
            continue;
        };
        let id = assignment.run.id.clone();
        let epoch = assignment.epoch;
        let (sender, receiver) = mpsc::sync_channel(1);
        let task = Arc::clone(&execute);
        thread::spawn(move || {
            let _ = sender.send(task(assignment.run));
        });
        loop {
            match receiver.recv_timeout(Duration::from_secs(1)) {
                Ok(Ok(WorkerResult::Completed(output))) => {
                    let _outcome = complete(&endpoint, &worker, id.clone(), epoch, output)?;
                    break;
                }
                Ok(Ok(WorkerResult::Waiting(wait_request))) => {
                    let _outcome = wait(&endpoint, &worker, id.clone(), epoch, wait_request)?;
                    break;
                }
                Ok(Err(message)) => {
                    let _outcome = fail(&endpoint, &worker, id.clone(), epoch, message)?;
                    break;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => heartbeat(&endpoint, &worker)?,
                Err(mpsc::RecvTimeoutError::Disconnected) => return Err(ControlError::State),
            }
        }
    }
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

fn register(endpoint: &Endpoint, worker: &str) -> Result<(), ControlError> {
    ok(request(
        endpoint,
        Request::Register {
            worker: worker.to_owned(),
            pid: std::process::id(),
            token: endpoint.token.clone(),
        },
    )?)
}
fn heartbeat(endpoint: &Endpoint, worker: &str) -> Result<(), ControlError> {
    ok(request(
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
        Response::Assignment { run } => Ok(run),
        Response::Error { message } => Err(ControlError::Rejected { message }),
        _ => Err(ControlError::State),
    }
}
fn complete(
    endpoint: &Endpoint,
    worker: &str,
    id: String,
    epoch: u64,
    output: u32,
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
