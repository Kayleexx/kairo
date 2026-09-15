use std::time::Instant;

use crate::{
    Response, RunPlan, RunStatus,
    history::{AssignmentReason, RunOutcome, now_ms},
    leases::PREFERRED_WORKER_WINDOW_MS,
    server::State,
};

/// a canceled run's report is finalized as `Canceled`, not whatever the worker reported.
pub(crate) fn finish(
    state: &mut State,
    worker: &str,
    id: String,
    epoch: u64,
    status: RunStatus,
) -> Response {
    let owns_running = matches!(
        state.runs.get(&id),
        Some(RunStatus::Running{worker: assigned, epoch: assigned_epoch})
            if assigned == worker && *assigned_epoch == epoch
    );
    let owns_cancelled = matches!(
        state.runs.get(&id),
        Some(RunStatus::CancelRequested{worker: assigned, epoch: assigned_epoch})
            if assigned == worker && *assigned_epoch == epoch
    );
    if !owns_running && !owns_cancelled {
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
    state.dirty = true;
    if owns_cancelled {
        state.record_outcome(&id, worker, epoch, RunOutcome::Canceled);
        state.runs.insert(id, RunStatus::Canceled);
        Response::Canceled
    } else {
        if let Some(outcome) = outcome_of(&status) {
            state.record_outcome(&id, worker, epoch, outcome);
        }
        state.runs.insert(id, status);
        Response::Ok
    }
}

/// a group boundary was reached: the run goes back through the queue for its next group,
/// carrying where to resume from and (once, from the first group) the run's durability plan.
/// Fenced identically to `finish` -- a stale epoch is rejected the same way a stale completion
/// would be.
#[allow(clippy::too_many_arguments)]
pub(crate) fn yield_group(
    state: &mut State,
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
) -> Response {
    let owns_running = matches!(
        state.runs.get(&id),
        Some(RunStatus::Running{worker: assigned, epoch: assigned_epoch})
            if assigned == worker && *assigned_epoch == epoch
    );
    let owns_cancelled = matches!(
        state.runs.get(&id),
        Some(RunStatus::CancelRequested{worker: assigned, epoch: assigned_epoch})
            if assigned == worker && *assigned_epoch == epoch
    );
    if !owns_running && !owns_cancelled {
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
    state.dirty = true;
    if owns_cancelled {
        state.record_outcome(&id, worker, epoch, RunOutcome::Canceled);
        state.runs.insert(id, RunStatus::Canceled);
        return Response::Canceled;
    }
    let Some(request) = state.requests.get_mut(&id) else {
        return Response::Error {
            message: "run request is missing".into(),
        };
    };
    request.resume = Some(crate::GroupResume {
        from_index: next_index,
        artifact_hash,
        artifact_backend,
    });
    if plan.is_some() {
        request.plan = plan;
    }
    if shape.is_some() {
        request.shape = shape;
    }
    request.preferred_worker = target_worker.clone();
    request.preferred_deadline_ms = target_worker
        .is_some()
        .then(|| now_ms().saturating_add(PREFERRED_WORKER_WINDOW_MS));
    let queued_request = request.clone();
    state.runs.insert(id.clone(), RunStatus::Queued);
    state.queued.push_back(queued_request);
    state.record_queued(
        &id,
        AssignmentReason::ReassignedAfterGroupYield { target_had_cache },
    );
    Response::Ok
}

/// only `Completed`/`Failed` are real terminal outcomes; `Waiting` is not an outcome.
fn outcome_of(status: &RunStatus) -> Option<RunOutcome> {
    match status {
        RunStatus::Completed { .. } => Some(RunOutcome::Completed),
        RunStatus::Failed { .. } => Some(RunOutcome::Failed),
        _ => None,
    }
}
