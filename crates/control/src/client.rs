use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpStream,
    sync::{Arc, mpsc},
    thread,
    time::Duration,
};

use crate::{ControlError, Endpoint, Request, Response, RunRequest, RunStatus, Snapshot};

const MAX_MESSAGE_BYTES: u64 = 1024 * 1024;

pub fn submit(endpoint: &Endpoint, run: RunRequest) -> Result<(), ControlError> {
    ok(request(
        endpoint,
        Request::Submit {
            token: endpoint.token.clone(),
            run,
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

pub fn worker_loop(
    endpoint: Endpoint,
    worker: String,
    execute: impl Fn(RunRequest) -> Result<u32, String> + Send + Sync + 'static,
) -> Result<(), ControlError> {
    register(&endpoint, &worker)?;
    let execute = Arc::new(execute);
    loop {
        heartbeat(&endpoint, &worker)?;
        let Some(run) = next(&endpoint, &worker)? else {
            thread::sleep(Duration::from_millis(100));
            continue;
        };
        let id = run.id.clone();
        let (sender, receiver) = mpsc::sync_channel(1);
        let task = Arc::clone(&execute);
        thread::spawn(move || {
            let _ = sender.send(task(run));
        });
        loop {
            match receiver.recv_timeout(Duration::from_secs(1)) {
                Ok(Ok(output)) => {
                    complete(&endpoint, &worker, id.clone(), output)?;
                    break;
                }
                Ok(Err(message)) => {
                    fail(&endpoint, &worker, id.clone(), message)?;
                    break;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => heartbeat(&endpoint, &worker)?,
                Err(mpsc::RecvTimeoutError::Disconnected) => return Err(ControlError::State),
            }
        }
    }
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
fn next(endpoint: &Endpoint, worker: &str) -> Result<Option<RunRequest>, ControlError> {
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
    output: u32,
) -> Result<(), ControlError> {
    ok(request(
        endpoint,
        Request::Complete {
            worker: worker.to_owned(),
            token: endpoint.token.clone(),
            id,
            output,
        },
    )?)
}
fn fail(
    endpoint: &Endpoint,
    worker: &str,
    id: String,
    message: String,
) -> Result<(), ControlError> {
    ok(request(
        endpoint,
        Request::Fail {
            worker: worker.to_owned(),
            token: endpoint.token.clone(),
            id,
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
