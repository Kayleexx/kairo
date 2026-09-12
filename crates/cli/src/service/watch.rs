use std::{path::Path, thread, time::Duration};

use kairo_runtime::inspect_cell;

use crate::{CliError, Result, status};

pub(super) fn output(endpoint: &kairo_control::Endpoint, id: &str, state: &Path) -> Result<u32> {
    let mut previous = String::new();
    let mut completed = 0;
    let mut owner: Option<(String, u64)> = None;
    let mut loss_reported = false;
    loop {
        let inspection = inspect_cell(state).ok();
        if let Some(inspection) = &inspection {
            for component in inspection.components.iter().skip(completed) {
                if component.output.is_none() {
                    break;
                }
                status("32", "✓", &component.name);
                completed += 1;
            }
        }
        let current = match kairo_control::status(endpoint, id.to_owned())? {
            Some(kairo_control::RunStatus::CancelRequested)
            | Some(kairo_control::RunStatus::Canceled) => {
                status("31", "✕", "run cancelled");
                return Err(CliError::Control(kairo_control::ControlError::State));
            }
            Some(kairo_control::RunStatus::Queued) => {
                if let Some((worker, _)) = &owner
                    && !loss_reported
                {
                    status(
                        "33",
                        "!",
                        &format!("{worker} lost · waiting for safe takeover"),
                    );
                    loss_reported = true;
                }
                "queued".to_owned()
            }
            Some(kairo_control::RunStatus::Running { worker, epoch }) => {
                if let Some((previous_worker, previous_epoch)) = &owner
                    && (previous_worker != &worker || *previous_epoch < epoch)
                {
                    let checkpoint = inspection.as_ref().is_some_and(|inspection| {
                        inspection
                            .components
                            .iter()
                            .any(|step| step.checkpoint.is_some())
                    });
                    status(
                        "36",
                        "↻",
                        &format!(
                            "recovered on {worker}{}",
                            if checkpoint { " from checkpoint" } else { "" }
                        ),
                    );
                }
                owner = Some((worker.clone(), epoch));
                format!("running on {worker}")
            }
            Some(kairo_control::RunStatus::Waiting { reason }) => wait(&reason),
            Some(kairo_control::RunStatus::Completed { output, .. }) => return Ok(output),
            Some(kairo_control::RunStatus::Failed { message }) => {
                return Err(CliError::Control(kairo_control::ControlError::Rejected {
                    message,
                }));
            }
            None => return Err(CliError::Control(kairo_control::ControlError::State)),
        };
        if current != previous {
            status("36", "●", &current);
            previous = current;
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn wait(reason: &str) -> String {
    reason.strip_prefix("signal:").map_or_else(
        || "waiting safely for its timer · worker released".to_owned(),
        |signal| format!("waiting for {signal} · worker released"),
    )
}
