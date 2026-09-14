use std::time::Instant;

use crate::{Response, RunStatus, server::State};

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
        state.runs.insert(id, RunStatus::Canceled);
        Response::Canceled
    } else {
        state.runs.insert(id, status);
        Response::Ok
    }
}
