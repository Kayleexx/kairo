use std::{
    io::Write,
    net::TcpStream,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use crate::{ControlError, Request, Response, RunSnapshot, RunStatus, Snapshot, WorkerSnapshot};

use super::{MAX_QUEUE, State};

pub(super) fn handle(
    mut stream: TcpStream,
    token: &str,
    state: Arc<Mutex<State>>,
    shutdown: Arc<AtomicBool>,
    assignments: Arc<Condvar>,
) -> Result<(), ControlError> {
    let request: Request = crate::protocol_io::read(&mut stream)?;
    let response = dispatch(request, token, &state, &shutdown, &assignments);
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
    assignments: &Condvar,
) -> Response {
    if request.token() != expected {
        return Response::Error {
            message: "authentication failed".into(),
        };
    }
    if let Request::Next { worker, .. } = request {
        return super::next::wait(worker, shared, shutdown, assignments);
    }
    let Ok(mut state) = shared.lock() else {
        return Response::Error {
            message: "control state is unavailable".into(),
        };
    };
    let queued = state.queued.len();
    let live_assignments = state.live_assignments.len();
    let response = match request {
        Request::Register { worker, pid, .. } => super::worker::register(&mut state, worker, pid),
        Request::Heartbeat { worker, .. } => super::worker::heartbeat(&mut state, worker),
        Request::Next { .. } => Response::Error {
            message: "assignment request was not long-polled".into(),
        },
        Request::LiveEdgeBegin {
            worker,
            session_id,
            id,
            edge_id,
            epoch,
            producer_group,
            consumer_group,
            consumer_worker,
            ..
        } => match state.begin_live_edge(
            session_id.clone(),
            id.clone(),
            edge_id.clone(),
            epoch,
            producer_group,
            worker.clone(),
            consumer_group,
            consumer_worker,
        ) {
            Ok(()) => match state.live_edge_event(
                &session_id,
                &id,
                &edge_id,
                crate::LiveEdgeParticipant::Producer,
                producer_group,
                &worker,
                epoch,
                crate::LiveEdgeState::Assigned,
                None,
            ) {
                Ok(()) => Response::Ok,
                Err(message) => Response::Error { message },
            },
            Err(message) => Response::Error { message },
        },
        Request::LiveEdgeReady {
            worker,
            session_id,
            id,
            edge_id,
            epoch,
            producer_group,
            endpoint,
            ..
        } => match state.live_edge_event(
            &session_id,
            &id,
            &edge_id,
            crate::LiveEdgeParticipant::Producer,
            producer_group,
            &worker,
            epoch,
            crate::LiveEdgeState::Ready,
            Some(endpoint),
        ) {
            Ok(()) => Response::Ok,
            Err(message) => Response::Error { message },
        },
        Request::LiveEdgeStreaming {
            worker,
            session_id,
            id,
            edge_id,
            epoch,
            consumer_group,
            ..
        } => match state.live_edge_event(
            &session_id,
            &id,
            &edge_id,
            crate::LiveEdgeParticipant::Consumer,
            consumer_group,
            &worker,
            epoch,
            crate::LiveEdgeState::Streaming,
            None,
        ) {
            Ok(()) => Response::Ok,
            Err(message) => Response::Error { message },
        },
        Request::LiveEdgeComplete {
            worker,
            session_id,
            id,
            edge_id,
            epoch,
            group,
            participant,
            output,
            metrics,
            ..
        } => match state.complete_live_edge(
            &session_id,
            &id,
            &edge_id,
            participant,
            group,
            &worker,
            epoch,
            output,
            metrics,
        ) {
            Ok(()) => Response::Ok,
            Err(message) => Response::Error { message },
        },
        Request::LiveEdgeFail {
            worker,
            session_id,
            id,
            edge_id,
            epoch,
            group,
            participant,
            message,
            ..
        } => match state.fail_live_edge(
            &session_id,
            &id,
            &edge_id,
            participant,
            group,
            &worker,
            epoch,
            message,
        ) {
            Ok(()) => Response::Ok,
            Err(message) => Response::Error { message },
        },
        Request::LiveEdgeStatus { session_id, .. } => Response::LiveEdge {
            session: Box::new(state.live_edges.get(&session_id).cloned()),
        },
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
        Request::Yield {
            worker,
            id,
            epoch,
            next_index,
            artifact_hash,
            artifact_backend,
            plan,
            shape,
            target_worker,
            target_had_cache,
            ..
        } => crate::finish::yield_group(
            &mut state,
            &worker,
            id,
            epoch,
            next_index,
            artifact_hash,
            artifact_backend,
            plan,
            shape,
            target_worker,
            target_had_cache,
        ),
        Request::Submit { run, lineage, .. } => {
            // a run a service restart already requeued (`State::load`'s `ResumedAfterRestart`)
            // is legitimately resubmitted by the exact same `kairo run --state <path>` retry
            // that crashed mid-flight -- accept it as a no-op rather than rejecting a resume the
            // caller has every right to make. A matching request remains idempotent after a
            // worker has already claimed the reconstructed run: it attaches to that run rather
            // than creating a second owner. A different request with the same id is rejected.
            if let Some(existing) = state.requests.get(&run.id) {
                if existing.workflow == run.workflow && existing.state == run.state {
                    Response::Ok
                } else {
                    Response::Error {
                        message: format!("run `{}` already exists", run.id),
                    }
                }
            } else if state.runs.contains_key(&run.id) {
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
                if let Some(lineage) = lineage {
                    state.lineages.insert(run.id.clone(), *lineage);
                }
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
        Request::Forget { id, .. } => {
            if state.forget(&id) {
                Response::Ok
            } else {
                Response::Error {
                    message: format!("run `{id}` cannot be forgotten (not found or not finished)"),
                }
            }
        }
        Request::Status { id, .. } => Response::Status {
            status: state.runs.get(&id).cloned(),
        },
        Request::Snapshot { .. } => {
            let _ = crate::leases::reclaim_expired(&mut state);
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
                            shape: state.requests.get(id).and_then(|run| run.shape.clone()),
                        })
                        .collect(),
                    live_edges: state.live_edges.values().cloned().collect(),
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
        Ok(()) => {
            if state.queued.len() > queued
                || state.live_assignments.len() > live_assignments
                || shutdown.load(Ordering::Relaxed)
            {
                assignments.notify_all();
            }
            response
        }
        Err(error) => Response::Error {
            message: error.to_string(),
        },
    }
}
