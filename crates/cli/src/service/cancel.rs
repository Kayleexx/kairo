use std::{collections::HashMap, path::Path};

use crate::{CliError, Result};

pub(crate) fn cancel(requested: &str) -> Result<()> {
    let endpoint = kairo_control::load_endpoint(Path::new(".kairo"))?;
    let run = resolve_run(&endpoint, requested)?;
    kairo_control::cancel(&endpoint, run.clone())?;
    println!("cancel requested · {run}");
    Ok(())
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
            is_cancelable(&run.status)
                && workflows
                    .get(&run.id)
                    .is_some_and(|workflow| workflow == requested)
        })
        .map(|run| run.id)
        .collect();
    match candidates.as_slice() {
        [run] => Ok(run.clone()),
        [] => Err(rejected(&format!(
            "no cancelable run found for `{requested}`; use `kairo runs`"
        ))),
        _ => Err(rejected(&format!(
            "multiple `{requested}` runs can be canceled; choose one: {}",
            candidates.join(", ")
        ))),
    }
}

fn is_cancelable(status: &kairo_control::RunStatus) -> bool {
    !matches!(
        status,
        kairo_control::RunStatus::Completed { .. }
            | kairo_control::RunStatus::Failed { .. }
            | kairo_control::RunStatus::Canceled
    )
}

fn rejected(message: &str) -> CliError {
    CliError::Control(kairo_control::ControlError::Rejected {
        message: message.to_owned(),
    })
}
