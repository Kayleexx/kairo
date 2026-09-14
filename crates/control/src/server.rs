use crate::{
    ControlError, Endpoint, Request, Response, RunRequest, RunSnapshot, RunStatus, Snapshot,
    WorkerSnapshot,
};
use getrandom::fill;
use std::{
    collections::{BTreeMap, VecDeque},
    fs,
    io::Write,
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
const MAX_QUEUE: usize = 1024;
pub(crate) struct State {
    pub(crate) directory: PathBuf,
    pub(crate) workers: BTreeMap<String, Worker>,
    pub(crate) queued: VecDeque<RunRequest>,
    pub(crate) requests: BTreeMap<String, RunRequest>,
    pub(crate) runs: BTreeMap<String, RunStatus>,
    pub(crate) epochs: BTreeMap<String, u64>,
    pub(crate) waiting: BTreeMap<String, crate::WaitRequest>,
    pub(crate) history: BTreeMap<String, Vec<crate::history::RunEvent>>,
    pub(crate) pending_reason: BTreeMap<String, crate::history::AssignmentReason>,
    pub(crate) dirty: bool,
}
pub(crate) struct Worker {
    pub(crate) busy: bool,
    pub(crate) last_seen: Instant,
    pub(crate) pid: u32,
}
pub struct Server {
    listener: TcpListener,
    endpoint: Endpoint,
    state: Arc<Mutex<State>>,
    shutdown: Arc<AtomicBool>,
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
        let state = State::load(directory)?;
        Ok(Self {
            listener,
            endpoint,
            state: Arc::new(Mutex::new(state)),
            shutdown: Arc::new(AtomicBool::new(false)),
        })
    }
    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }
    pub fn serve(&self) -> Result<(), ControlError> {
        self.serve_while(|| !self.shutdown.load(Ordering::Relaxed))
    }
    pub fn serve_until(&self, shutdown: &AtomicBool) -> Result<(), ControlError> {
        self.serve_while(|| !shutdown.load(Ordering::Relaxed))
    }
    fn serve_while(&self, keep: impl Fn() -> bool) -> Result<(), ControlError> {
        while keep() {
            if let Ok(mut state) = self.state.lock()
                && crate::leases::resume_waiting(&mut state)
            {
                let _ = state.persist();
            }
            match self.listener.accept() {
                Ok((stream, _)) => {
                    let state = Arc::clone(&self.state);
                    let token = self.endpoint.token.clone();
                    let shutdown = Arc::clone(&self.shutdown);
                    thread::spawn(move || {
                        let _ = handle(stream, &token, state, shutdown);
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
    shutdown: Arc<AtomicBool>,
) -> Result<(), ControlError> {
    let request: Request = crate::protocol_io::read(&mut stream)?;
    let response = dispatch(request, token, &state, &shutdown);
    let bytes =
        serde_json::to_vec(&response).map_err(|source| ControlError::Protocol { source })?;
    stream
        .write_all(&bytes)
        .map_err(|source| ControlError::Io { source })?;
    stream
        .write_all(b"\n")
        .map_err(|source| ControlError::Io { source })
}
fn dispatch(
    request: Request,
    expected: &str,
    shared: &Arc<Mutex<State>>,
    shutdown: &AtomicBool,
) -> Response {
    if request.token() != expected {
        return Response::Error {
            message: "authentication failed".into(),
        };
    }
    let Ok(mut state) = shared.lock() else {
        return Response::Error {
            message: "control state is unavailable".into(),
        };
    };
    let response = match request {
        Request::Register { worker, pid, .. } => match state.workers.entry(worker) {
            std::collections::btree_map::Entry::Occupied(mut entry)
                if entry.get().last_seen.elapsed() > Duration::from_secs(3) =>
            {
                entry.insert(Worker {
                    busy: false,
                    last_seen: Instant::now(),
                    pid,
                });
                Response::Ok
            }
            std::collections::btree_map::Entry::Occupied(entry) => Response::Error {
                message: format!("worker `{}` is already registered", entry.key()),
            },
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(Worker {
                    busy: false,
                    last_seen: Instant::now(),
                    pid,
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
            crate::leases::reclaim_expired(&mut state);
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
            let run = state.queued.pop_front();
            if let Some(run) = &run {
                if let Some(item) = state.workers.get_mut(&worker) {
                    item.busy = true;
                }
                let epoch = {
                    let epoch = state.epochs.entry(run.id.clone()).or_insert(0);
                    *epoch = epoch.saturating_add(1);
                    *epoch
                };
                state.runs.insert(
                    run.id.clone(),
                    RunStatus::Running {
                        worker: worker.clone(),
                        epoch,
                    },
                );
                state.record_assigned(&run.id, &worker, epoch);
                state.dirty = true;
                return Response::Assignment {
                    run: Some(crate::Assignment {
                        run: run.clone(),
                        epoch,
                    }),
                };
            }
            Response::Assignment { run: None }
        }
        Request::Complete {
            worker,
            id,
            epoch,
            output,
            ..
        } => crate::finish::finish(
            &mut state,
            &worker,
            id,
            epoch,
            RunStatus::Completed {
                output,
                worker: worker.clone(),
            },
        ),
        Request::Fail {
            worker,
            id,
            epoch,
            message,
            ..
        } => crate::finish::finish(
            &mut state,
            &worker,
            id,
            epoch,
            RunStatus::Failed { message },
        ),
        Request::Wait {
            worker,
            id,
            epoch,
            wait,
            ..
        } => {
            let response = crate::finish::finish(
                &mut state,
                &worker,
                id.clone(),
                epoch,
                RunStatus::Waiting {
                    reason: crate::leases::wait_reason(&wait),
                },
            );
            if matches!(response, Response::Ok) {
                state.waiting.insert(id, wait);
            }
            response
        }
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
                let waiting = run.wait.clone();
                state.runs.insert(
                    run.id.clone(),
                    waiting
                        .as_ref()
                        .map_or(RunStatus::Queued, |wait| RunStatus::Waiting {
                            reason: crate::leases::wait_reason(wait),
                        }),
                );
                state.requests.insert(run.id.clone(), run.clone());
                if let Some(wait) = waiting {
                    state.waiting.insert(run.id.clone(), wait);
                } else {
                    state.record_queued(&run.id, crate::history::AssignmentReason::Initial);
                    state.queued.push_back(run);
                }
                state.dirty = true;
                Response::Ok
            }
        }
        Request::Cancel { id, .. } => {
            if state.cancel(&id) {
                Response::Ok
            } else {
                Response::Error {
                    message: format!("run `{id}` cannot be canceled"),
                }
            }
        }
        Request::Status { id, .. } => Response::Status {
            status: state.runs.get(&id).cloned(),
        },
        Request::Snapshot { .. } => {
            crate::leases::reclaim_expired(&mut state);
            Response::Snapshot {
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
                            history: state.history.get(id).cloned().unwrap_or_default(),
                        })
                        .collect(),
                },
            }
        }
        Request::Signal { id, signal, .. } => match state.waiting.get(&id) {
            Some(crate::WaitRequest::Signal { name }) if name == &signal => {
                crate::leases::resume(&mut state, &id);
                Response::Ok
            }
            Some(_) => Response::Error {
                message: "run is not waiting for that signal".into(),
            },
            None => Response::Error {
                message: format!("run `{id}` does not exist"),
            },
        },
        Request::ChaosKill { worker, .. } => match state.workers.get(&worker) {
            Some(item) => crate::chaos::kill(&worker, item.pid),
            None => Response::Error {
                message: format!("worker `{worker}` is not registered"),
            },
        },
        Request::Shutdown { .. } => {
            shutdown.store(true, Ordering::Relaxed);
            Response::Ok
        }
    };
    match state.persist() {
        Ok(()) => response,
        Err(error) => Response::Error {
            message: error.to_string(),
        },
    }
}
