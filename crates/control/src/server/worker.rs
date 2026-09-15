use std::{collections::btree_map::Entry, time::Duration, time::Instant};

use crate::{Response, RunStatus};

use super::{State, Worker};

pub(super) fn register(state: &mut State, worker: String, pid: u32) -> Response {
    match state.workers.entry(worker) {
        Entry::Occupied(mut entry) if entry.get().last_seen.elapsed() > Duration::from_secs(3) => {
            entry.insert(Worker {
                busy: false,
                last_seen: Instant::now(),
                pid,
            });
            Response::Ok
        }
        Entry::Occupied(entry) => Response::Error {
            message: format!("worker `{}` is already registered", entry.key()),
        },
        Entry::Vacant(entry) => {
            entry.insert(Worker {
                busy: false,
                last_seen: Instant::now(),
                pid,
            });
            Response::Ok
        }
    }
}

pub(super) fn heartbeat(state: &mut State, worker: String) -> Response {
    match state.workers.get_mut(&worker) {
        Some(item) => {
            item.last_seen = Instant::now();
            Response::Ok
        }
        None => Response::Error {
            message: "worker is not registered".into(),
        },
    }
}

pub(super) fn next(state: &mut State, worker: String) -> Response {
    crate::leases::reclaim_expired(state);
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
    }
    let run = crate::leases::pop_assignable(state, &worker);
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
            run: Some(Box::new(crate::Assignment {
                run: run.clone(),
                epoch,
            })),
        };
    }
    Response::Assignment { run: None }
}
