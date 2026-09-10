use std::time::SystemTime;

use kairo_runtime::{CellStatus, StreamRunStatus};

use crate::Run;

pub(crate) fn activity(run: &Run) -> String {
    match run.service.as_ref() {
        Some(kairo_control::RunStatus::Queued) => "queued".to_owned(),
        Some(kairo_control::RunStatus::Running { worker, .. }) => format!("running · {worker}"),
        Some(kairo_control::RunStatus::Failed { message }) => format!("failed · {message}"),
        Some(kairo_control::RunStatus::Waiting { reason }) => wait(reason),
        Some(kairo_control::RunStatus::Completed { worker, .. }) if run.inspection.is_none() => {
            if worker.is_empty() {
                "completed".to_owned()
            } else {
                format!("completed · {worker}")
            }
        }
        _ => run.inspection.as_ref().map_or_else(
            || {
                run.stream.as_ref().map_or_else(
                    || "waiting for worker".to_owned(),
                    |stream| match &stream.status {
                        StreamRunStatus::Running => "streaming".to_owned(),
                        StreamRunStatus::Completed => "completed".to_owned(),
                        StreamRunStatus::Failed(message) => format!("failed · {message}"),
                    },
                )
            },
            |inspection| status(&inspection.status).to_owned(),
        ),
    }
}

fn wait(reason: &str) -> String {
    if let Some(signal) = reason.strip_prefix("signal:") {
        return format!("waiting for {signal}");
    }
    if let Some(due) = reason
        .strip_prefix("timer:")
        .and_then(|value| value.parse::<u64>().ok())
    {
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(due, |value| {
                value.as_millis().min(u128::from(u64::MAX)) as u64
            });
        return format!("resumes in {}s", due.saturating_sub(now).div_ceil(1_000));
    }
    "waiting safely".to_owned()
}

pub(crate) fn status(status: &CellStatus) -> &'static str {
    match status {
        CellStatus::Completed { .. } => "completed",
        CellStatus::Ready { .. } => "ready",
        CellStatus::Interrupted { .. } => "interrupted",
        CellStatus::CheckpointPending { .. } => "saving checkpoint",
        CellStatus::Finalizing => "finalizing",
    }
}
