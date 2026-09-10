use std::{collections::HashMap, path::Path};

use crate::{CliError, Result};

pub(crate) fn signal(requested: &str, signal: Option<&str>) -> Result<()> {
    let endpoint = kairo_control::load_endpoint(Path::new(".kairo"))?;
    let run = resolve_run(&endpoint, requested)?;
    let signal = match signal {
        Some(signal) => signal.to_owned(),
        None => infer_signal(&endpoint, &run)?,
    };
    if signal.is_empty() || signal.len() > 128 || signal.chars().any(char::is_control) {
        return Err(rejected("signal must be 1–128 printable characters"));
    }
    kairo_control::signal(&endpoint, run, signal.clone())?;
    println!("signal accepted · {signal}");
    Ok(())
}

fn infer_signal(endpoint: &kairo_control::Endpoint, run: &str) -> Result<String> {
    match kairo_control::status(endpoint, run.to_owned())? {
        Some(kairo_control::RunStatus::Waiting { reason }) => reason
            .strip_prefix("signal:")
            .map(str::to_owned)
            .ok_or_else(|| rejected("this run is waiting for a timer; it resumes automatically")),
        _ => Err(rejected(
            "no pending signal found; use `kairo signal RUN SIGNAL`",
        )),
    }
}

fn resolve_run(endpoint: &kairo_control::Endpoint, requested: &str) -> Result<String> {
    if kairo_control::status(endpoint, requested.to_owned())?.is_some() {
        return Ok(requested.to_owned());
    }
    let workflows = crate::state::discover()?
        .into_iter()
        .filter_map(|run| {
            kairo_runtime::inspect_cell(&run.path)
                .ok()
                .and_then(|inspection| inspection.name.map(|name| (run.name, name)))
        })
        .collect::<HashMap<_, _>>();
    let candidates: Vec<_> = kairo_control::snapshot(endpoint)?
        .runs
        .into_iter()
        .filter(|run| {
            matches!(run.status, kairo_control::RunStatus::Waiting { ref reason } if reason.starts_with("signal:"))
                && workflows.get(&run.id).is_some_and(|workflow| workflow == requested)
        })
        .map(|run| run.id)
        .collect();
    match candidates.as_slice() {
        [run] => Ok(run.clone()),
        [] => Err(rejected(&format!(
            "no waiting run found for `{requested}`; use `kairo runs`"
        ))),
        _ => Err(rejected(&format!(
            "multiple `{requested}` runs are waiting; choose one: {}",
            candidates.join(", ")
        ))),
    }
}

fn rejected(message: &str) -> CliError {
    CliError::Control(kairo_control::ControlError::Rejected {
        message: message.to_owned(),
    })
}
