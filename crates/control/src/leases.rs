use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::{
    RunRequest, RunStatus,
    history::{AssignmentReason, RunOutcome},
    server::State,
};

/// how long a queued ExecutionGroup continuation waits for its chosen placement before any
/// capable worker may take it instead -- an anti-starvation fallback, not a scheduling policy.
pub(crate) const PREFERRED_WORKER_WINDOW_MS: u64 = 2000;

/// pops the first queued run assignable to `worker`: a run reserved for `worker` specifically
/// (regardless of queue position) takes priority; otherwise the first run with no live
/// reservation for a different worker, in FIFO order.
pub(crate) fn pop_assignable(state: &mut State, worker: &str) -> Option<RunRequest> {
    if let Some(index) = state
        .queued
        .iter()
        .position(|run| run.preferred_worker.as_deref() == Some(worker))
    {
        return state.queued.remove(index);
    }
    let now = crate::history::now_ms();
    let index = state
        .queued
        .iter()
        .position(|run| match &run.preferred_worker {
            None => true,
            Some(_) => run
                .preferred_deadline_ms
                .is_some_and(|deadline| now >= deadline),
        })?;
    state.queued.remove(index)
}

pub(crate) fn reclaim_expired(state: &mut State) -> bool {
    let expired: Vec<_> = state
        .workers
        .iter()
        .filter(|(_, worker)| worker.last_seen.elapsed() > Duration::from_secs(3))
        .map(|(id, _)| id.clone())
        .collect();
    let changed = !expired.is_empty();
    for worker in expired {
        if let Some(item) = state.workers.get_mut(&worker) {
            item.busy = false;
        }
        let mut runs = state.invalidate_live_edges_for_worker(&worker);
        runs.extend(
            state
                .runs
                .iter()
                .filter_map(|(id, status)| {
                    matches!(status, RunStatus::Running { worker: owner, .. } if owner == &worker)
                        .then_some(id.clone())
                })
                .collect::<Vec<_>>(),
        );
        runs.sort();
        runs.dedup();
        for id in runs {
            if let Some(run) = state.runs.get_mut(&id) {
                *run = RunStatus::Queued;
            }
            if let Some(request) = state.requests.get(&id) {
                state.queued.push_back(request.clone());
            }
            state.record_queued(&id, AssignmentReason::ReassignedAfterLeaseExpiry);
            state.dirty = true;
        }
        // finalize, don't requeue: re-running a canceled workflow would be wrong.
        let canceled: Vec<_> = state
            .runs
            .iter()
            .filter_map(|(id, status)| match status {
                RunStatus::CancelRequested {
                    worker: owner,
                    epoch,
                } if owner == &worker => Some((id.clone(), *epoch)),
                _ => None,
            })
            .collect();
        for (id, epoch) in canceled {
            if let Some(run) = state.runs.get_mut(&id) {
                *run = RunStatus::Canceled;
            }
            state.record_outcome(&id, &worker, epoch, RunOutcome::Canceled);
            state.dirty = true;
        }
    }
    changed
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
    state.record_queued(id, AssignmentReason::ResumedAfterWait);
    state.dirty = true;
}

pub(crate) fn wait_reason(wait: &crate::WaitRequest) -> String {
    match wait {
        crate::WaitRequest::Timer { due_ms } => format!("timer:{due_ms}"),
        crate::WaitRequest::Signal { name } => format!("signal:{name}"),
    }
}
