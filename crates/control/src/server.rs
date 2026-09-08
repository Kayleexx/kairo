use crate::{
    ControlError, Endpoint, Request, Response, RunRequest, RunSnapshot, RunStatus, Snapshot,
    WorkerSnapshot,
};
use getrandom::fill;
use std::{
    collections::{BTreeMap, VecDeque},
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
const MAX_MESSAGE_BYTES: u64 = 1024 * 1024;
const MAX_QUEUE: usize = 1024;
#[derive(Default)]
struct State {
    workers: BTreeMap<String, Worker>,
    queued: VecDeque<RunRequest>,
    runs: BTreeMap<String, RunStatus>,
}
struct Worker {
    busy: bool,
    last_seen: Instant,
}
pub struct Server {
    listener: TcpListener,
    endpoint: Endpoint,
    state: Arc<Mutex<State>>,
}
impl Server {
    pub fn start(directory: &Path) -> Result<Self, ControlError> {
        fs::create_dir_all(directory).map_err(|source| ControlError::CreateDirectory {
            path: directory.to_path_buf(),
            source,
        })?;
        let listener =
            TcpListener::bind("127.0.0.1:0").map_err(|source| ControlError::Bind { source })?;
        listener
            .set_nonblocking(true)
            .map_err(|source| ControlError::Io { source })?;
        let mut bytes = [0; 32];
        fill(&mut bytes).map_err(|source| ControlError::Random { source })?;
        let endpoint = Endpoint {
            address: listener
                .local_addr()
                .map_err(|source| ControlError::Io { source })?,
            token: bytes.iter().map(|byte| format!("{byte:02x}")).collect(),
        };
        fs::write(
            directory.join("control.json"),
            serde_json::to_vec(&endpoint).map_err(|source| ControlError::Protocol { source })?,
        )
        .map_err(|source| ControlError::Io { source })?;
        Ok(Self {
            listener,
            endpoint,
            state: Arc::new(Mutex::new(State::default())),
        })
    }
    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }
    pub fn serve(&self) -> Result<(), ControlError> {
        self.serve_while(|| true)
    }
    pub fn serve_until(&self, shutdown: &AtomicBool) -> Result<(), ControlError> {
        self.serve_while(|| !shutdown.load(Ordering::Relaxed))
    }
    fn serve_while(&self, keep: impl Fn() -> bool) -> Result<(), ControlError> {
        while keep() {
            match self.listener.accept() {
                Ok((stream, _)) => {
                    let state = Arc::clone(&self.state);
                    let token = self.endpoint.token.clone();
                    thread::spawn(move || {
                        let _ = handle(stream, &token, state);
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(20))
                }
                Err(source) => return Err(ControlError::Io { source }),
            }
        }
        Ok(())
    }
}
pub fn load_endpoint(directory: &Path) -> Result<Endpoint, ControlError> {
    let text = fs::read_to_string(directory.join("control.json")).map_err(|source| {
        if source.kind() == std::io::ErrorKind::NotFound {
            ControlError::Unavailable
        } else {
            ControlError::Io { source }
        }
    })?;
    serde_json::from_str(&text).map_err(|source| ControlError::Protocol { source })
}
fn handle(
    mut stream: TcpStream,
    token: &str,
    state: Arc<Mutex<State>>,
) -> Result<(), ControlError> {
    let request: Request = read(&mut stream)?;
    let response = dispatch(request, token, &state);
    let bytes =
        serde_json::to_vec(&response).map_err(|source| ControlError::Protocol { source })?;
    stream
        .write_all(&bytes)
        .map_err(|source| ControlError::Io { source })?;
    stream
        .write_all(b"\n")
        .map_err(|source| ControlError::Io { source })
}
fn dispatch(request: Request, expected: &str, shared: &Arc<Mutex<State>>) -> Response {
    if !auth(&request, expected) {
        return Response::Error {
            message: "authentication failed".into(),
        };
    }
    let Ok(mut state) = shared.lock() else {
        return Response::Error {
            message: "control state is unavailable".into(),
        };
    };
    match request {
        Request::Register { worker, .. } => match state.workers.entry(worker) {
            std::collections::btree_map::Entry::Occupied(entry) => Response::Error {
                message: format!("worker `{}` is already registered", entry.key()),
            },
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(Worker {
                    busy: false,
                    last_seen: Instant::now(),
                });
                Response::Ok
            }
        },
        Request::Heartbeat { worker, .. } => match state.workers.get_mut(&worker) {
            Some(item) => {
                item.last_seen = Instant::now();
                Response::Ok
            }
            None => Response::Error {
                message: "worker is not registered".into(),
            },
        },
        Request::Next { worker, .. } => {
            let Some(item) = state.workers.get_mut(&worker) else {
                return Response::Error {
                    message: "worker is not registered".into(),
                };
            };
            item.last_seen = Instant::now();
            if item.busy {
                return Response::Error {
                    message: "worker already has a run".into(),
                };
            };
            let _ = item;
            let run = state.queued.pop_front();
            if let Some(run) = &run {
                if let Some(item) = state.workers.get_mut(&worker) {
                    item.busy = true;
                }
                state
                    .runs
                    .insert(run.id.clone(), RunStatus::Running { worker });
            }
            Response::Assignment { run }
        }
        Request::Complete {
            worker, id, output, ..
        } => finish(&mut state, &worker, id, RunStatus::Completed { output }),
        Request::Fail {
            worker,
            id,
            message,
            ..
        } => finish(&mut state, &worker, id, RunStatus::Failed { message }),
        Request::Submit { run, .. } => {
            if state.runs.contains_key(&run.id) {
                Response::Error {
                    message: format!("run `{}` already exists", run.id),
                }
            } else if state.queued.len() == MAX_QUEUE {
                Response::Error {
                    message: "run queue is full".into(),
                }
            } else {
                state.runs.insert(run.id.clone(), RunStatus::Queued);
                state.queued.push_back(run);
                Response::Ok
            }
        }
        Request::Status { id, .. } => Response::Status {
            status: state.runs.get(&id).cloned(),
        },
        Request::Snapshot { .. } => Response::Snapshot {
            snapshot: Snapshot {
                workers: state
                    .workers
                    .iter()
                    .map(|(id, item)| WorkerSnapshot {
                        id: id.clone(),
                        busy: item.busy,
                        healthy: item.last_seen.elapsed() <= Duration::from_secs(3),
                    })
                    .collect(),
                runs: state
                    .runs
                    .iter()
                    .map(|(id, status)| RunSnapshot {
                        id: id.clone(),
                        status: status.clone(),
                    })
                    .collect(),
            },
        },
    }
}
fn finish(state: &mut State, worker: &str, id: String, status: RunStatus) -> Response {
    if !matches!(state.runs.get(&id),Some(RunStatus::Running{worker:assigned})if assigned==worker) {
        return Response::Error {
            message: "worker does not own this run".into(),
        };
    }
    let Some(item) = state.workers.get_mut(worker) else {
        return Response::Error {
            message: "worker is not registered".into(),
        };
    };
    item.busy = false;
    item.last_seen = Instant::now();
    state.runs.insert(id, status);
    Response::Ok
}
fn auth(request: &Request, expected: &str) -> bool {
    match request {
        Request::Register { token, .. }
        | Request::Heartbeat { token, .. }
        | Request::Next { token, .. }
        | Request::Complete { token, .. }
        | Request::Fail { token, .. }
        | Request::Submit { token, .. }
        | Request::Status { token, .. }
        | Request::Snapshot { token } => token == expected,
    }
}
fn read<T: for<'a> serde::Deserialize<'a>>(stream: &mut TcpStream) -> Result<T, ControlError> {
    let mut bytes = Vec::new();
    BufReader::new(stream)
        .take(MAX_MESSAGE_BYTES + 1)
        .read_until(b'\n', &mut bytes)
        .map_err(|source| ControlError::Io { source })?;
    if bytes.len() as u64 > MAX_MESSAGE_BYTES {
        return Err(ControlError::TooLarge);
    }
    serde_json::from_slice(&bytes).map_err(|source| ControlError::Protocol { source })
}
