use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::{RunStatus, server::State};

pub(crate) fn reclaim_expired(state: &mut State) {
    let expired: Vec<_> = state
        .workers
        .iter()
        .filter(|(_, worker)| worker.last_seen.elapsed() > Duration::from_secs(3))
        .map(|(id, _)| id.clone())
        .collect();
    for worker in expired {
        if let Some(item) = state.workers.get_mut(&worker) {
            item.busy = false;
        }
        let runs: Vec<_> = state
            .runs
            .iter()
            .filter_map(|(id, status)| {
                matches!(status, RunStatus::Running { worker: owner, .. } if owner == &worker)
                    .then_some(id.clone())
            })
            .collect();
        for id in runs {
            if let Some(run) = state.runs.get_mut(&id) {
                *run = RunStatus::Queued;
            }
            if let Some(request) = state.requests.get(&id) {
                state.queued.push_back(request.clone());
            }
            state.dirty = true;
        }
    }
}

pub(crate) fn resume_waiting(state: &mut State) -> bool {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(u64::MAX, |time| {
            time.as_millis().min(u128::from(u64::MAX)) as u64
        });
    let ready: Vec<_> = state
        .waiting
        .iter()
        .filter_map(|(id, wait)| {
            matches!(wait, crate::WaitRequest::Timer { due_ms } if *due_ms <= now)
                .then_some(id.clone())
        })
        .collect();
    let changed = !ready.is_empty();
    for id in ready {
        resume(state, &id);
    }
    changed
}

pub(crate) fn resume(state: &mut State, id: &str) {
    let wait = state.waiting.remove(id);
    state.runs.insert(id.to_owned(), RunStatus::Queued);
    if let Some(run) = state.requests.get_mut(id) {
        run.wait = wait;
        state.queued.push_back(run.clone());
    }
    state.dirty = true;
}

pub(crate) fn wait_reason(wait: &crate::WaitRequest) -> String {
    match wait {
        crate::WaitRequest::Timer { due_ms } => format!("timer:{due_ms}"),
        crate::WaitRequest::Signal { name } => format!("signal:{name}"),
    }
}
