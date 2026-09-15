use std::path::Path;

use kairo_control::{AssignmentReason, RunEvent};

pub(super) fn status(
    name: &str,
) -> Result<Option<kairo_control::RunStatus>, kairo_control::ControlError> {
    let Some(endpoint) = live_endpoint()? else {
        return Ok(None);
    };
    // the endpoint file existing doesn't mean the service behind it is still alive (e.g. a
    // `kairo start` process killed without `kairo down` leaves a stale `control.json`) -- a
    // request against it can independently report `Unavailable`, and that must degrade the same
    // way a missing endpoint file already does, not fail the whole command.
    match kairo_control::status(&endpoint, name.to_owned()) {
        Ok(status) => Ok(status),
        Err(kairo_control::ControlError::Unavailable) => Ok(None),
        Err(error) => Err(error),
    }
}

/// the real worker-assignment history for this run, if a control service is reachable --
/// `None` when there is no live service to ask, never a guess.
pub(super) fn assignment_history(
    name: &str,
) -> Result<Option<Vec<RunEvent>>, kairo_control::ControlError> {
    let Some(endpoint) = live_endpoint()? else {
        return Ok(None);
    };
    let snapshot = match kairo_control::snapshot(&endpoint) {
        Ok(snapshot) => snapshot,
        Err(kairo_control::ControlError::Unavailable) => return Ok(None),
        Err(error) => return Err(error),
    };
    Ok(snapshot
        .runs
        .into_iter()
        .find(|run| run.id == name)
        .map(|run| run.history))
}

fn live_endpoint() -> Result<Option<kairo_control::Endpoint>, kairo_control::ControlError> {
    match kairo_control::load_endpoint(Path::new(".kairo")) {
        Ok(endpoint) => Ok(Some(endpoint)),
        Err(kairo_control::ControlError::Unavailable) => Ok(None),
        Err(error) => Err(error),
    }
}

/// prints one line per real assignment, oldest first -- only called once the caller has already
/// confirmed there is more than one, so this never prints a one-line "placement" section for the
/// overwhelming common case of a run that never moved. Each `Assigned` event corresponds 1:1 to
/// one real ExecutionGroup being picked up, in order, so its position in this filtered sequence
/// -- not a separately tracked id -- is the group's real index.
pub(super) fn print_placement(history: &[RunEvent]) {
    println!("\nplacement");
    let mut previous: Option<&str> = None;
    for (group, event) in history
        .iter()
        .filter_map(|event| match event {
            RunEvent::Assigned { worker, reason, .. } => Some((worker.as_str(), reason)),
            _ => None,
        })
        .enumerate()
    {
        let (worker, reason) = event;
        println!(
            "  group {group} · {}",
            transition_label(previous, worker, reason)
        );
        previous = Some(worker);
    }
}

fn transition_label(previous: Option<&str>, worker: &str, reason: &AssignmentReason) -> String {
    match reason {
        AssignmentReason::Initial => format!("{worker} · initial assignment"),
        AssignmentReason::ReassignedAfterLeaseExpiry => {
            format!("{worker} · worker lost, reassigned")
        }
        AssignmentReason::ResumedAfterWait => format!("{worker} · resumed after wait"),
        AssignmentReason::ResumedAfterRestart => {
            format!("{worker} · resumed after control-plane restart")
        }
        AssignmentReason::ReassignedAfterGroupYield { target_had_cache } => {
            // the group's *previous* worker, not this run's very first assignment -- a group
            // yield always has an immediately preceding `Assigned` event to compare against.
            let from = previous.unwrap_or(worker);
            let cache = if *target_had_cache {
                " · target had cache"
            } else {
                ""
            };
            if from == worker {
                format!("{worker} \u{2192} {worker} · reassigned after durable boundary{cache}")
            } else {
                format!("{from} \u{2192} {worker} · moved after durable boundary{cache}")
            }
        }
    }
}

pub(super) fn print(name: &str, status: kairo_control::RunStatus) {
    println!("{name}");
    match status {
        kairo_control::RunStatus::Queued => println!("  state · queued"),
        kairo_control::RunStatus::Running { worker, .. } => {
            println!("  state · running");
            println!("  worker · {worker}");
        }
        kairo_control::RunStatus::Waiting { reason } => {
            if let Some(signal) = reason.strip_prefix("signal:") {
                println!("  state · waiting for {signal}");
                println!("  worker · released");
                println!("\nnext · kairo signal {name}");
            } else {
                println!("  state · waiting for its timer");
                println!("  worker · released");
            }
        }
        kairo_control::RunStatus::Completed { output, worker } => {
            println!("  state · completed");
            if !worker.is_empty() {
                println!("  worker · {worker}");
            }
            println!("  output · {output}");
        }
        kairo_control::RunStatus::Failed { message } => {
            println!("  state · failed");
            println!("  error · {message}");
        }
        kairo_control::RunStatus::CancelRequested { .. } => {
            println!("  state · cancel requested");
        }
        kairo_control::RunStatus::Canceled => {
            println!("  state · canceled");
        }
    }
}

pub(super) fn print_state(name: &str, status: &kairo_control::RunStatus) -> bool {
    match status {
        kairo_control::RunStatus::Waiting { reason } => {
            if let Some(signal) = reason.strip_prefix("signal:") {
                println!("  state · waiting for {signal}");
                println!("  worker · released");
                println!("  next · kairo signal {name}");
            } else {
                println!("  state · waiting for its timer");
                println!("  worker · released");
            }
            true
        }
        kairo_control::RunStatus::Queued => {
            println!("  state · queued");
            true
        }
        kairo_control::RunStatus::Running { worker, .. } => {
            println!("  state · running on {worker}");
            true
        }
        kairo_control::RunStatus::Failed { message } => {
            println!("  state · failed");
            println!("  error · {message}");
            true
        }
        kairo_control::RunStatus::Completed { .. } => false,
        kairo_control::RunStatus::CancelRequested { .. } => {
            println!("  state · cancel requested");
            true
        }
        kairo_control::RunStatus::Canceled => {
            println!("  state · canceled");
            true
        }
    }
}
