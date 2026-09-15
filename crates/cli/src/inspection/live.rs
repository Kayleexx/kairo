use std::path::Path;

use kairo_control::{AssignmentReason, RunEvent};

pub(super) fn status(
    name: &str,
) -> Result<Option<kairo_control::RunStatus>, kairo_control::ControlError> {
    let endpoint = match kairo_control::load_endpoint(Path::new(".kairo")) {
        Ok(endpoint) => endpoint,
        Err(kairo_control::ControlError::Unavailable) => return Ok(None),
        Err(error) => return Err(error),
    };
    kairo_control::status(&endpoint, name.to_owned())
}

/// the real worker-assignment history for this run, if a control service is reachable --
/// `None` when there is no live service to ask, never a guess.
pub(super) fn assignment_history(
    name: &str,
) -> Result<Option<Vec<RunEvent>>, kairo_control::ControlError> {
    let endpoint = match kairo_control::load_endpoint(Path::new(".kairo")) {
        Ok(endpoint) => endpoint,
        Err(kairo_control::ControlError::Unavailable) => return Ok(None),
        Err(error) => return Err(error),
    };
    let snapshot = kairo_control::snapshot(&endpoint)?;
    Ok(snapshot
        .runs
        .into_iter()
        .find(|run| run.id == name)
        .map(|run| run.history))
}

/// prints one line per real assignment, oldest first -- only called once the caller has already
/// confirmed there is more than one, so this never prints a one-line "placement" section for the
/// overwhelming common case of a run that never moved.
pub(super) fn print_placement(history: &[RunEvent]) {
    println!("\nplacement");
    for event in history {
        if let RunEvent::Assigned { worker, reason, .. } = event {
            println!("  {worker} · {}", reason_label(reason));
        }
    }
}

fn reason_label(reason: &AssignmentReason) -> String {
    match reason {
        AssignmentReason::Initial => "initial assignment".to_owned(),
        AssignmentReason::ReassignedAfterLeaseExpiry => "worker lost · reassigned".to_owned(),
        AssignmentReason::ResumedAfterWait => "resumed after wait".to_owned(),
        AssignmentReason::ResumedAfterRestart => "resumed after control-plane restart".to_owned(),
        AssignmentReason::ReassignedAfterGroupYield { target_had_cache } => {
            if *target_had_cache {
                "execution group moved · target had cache".to_owned()
            } else {
                "execution group moved".to_owned()
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
