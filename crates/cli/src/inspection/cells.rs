use kairo_runtime::{CellInspection, JournalError};

use crate::state::LocalCell;

use super::{InspectionError, inventory, status_marker, status_summary, stream};

#[derive(serde::Serialize)]
struct RunSummary {
    name: String,
    workflow: String,
    kind: &'static str,
    state: String,
}

fn print_cells_json(
    ready: &[&(LocalCell, CellInspection)],
    streams: &[&(LocalCell, kairo_runtime::StreamRunInspection)],
    unavailable: &[(LocalCell, inventory::RunReadError)],
) -> Result<(), InspectionError> {
    let mut runs: Vec<RunSummary> = Vec::new();
    for (run, inspection) in ready {
        runs.push(RunSummary {
            name: run.name.clone(),
            workflow: inspection.name.clone().unwrap_or_default(),
            kind: "cell",
            state: status_summary(&inspection.status),
        });
    }
    for (run, inspection) in streams {
        runs.push(RunSummary {
            name: run.name.clone(),
            workflow: inspection.workflow.clone(),
            kind: "stream",
            state: stream::status(&inspection.status).to_owned(),
        });
    }
    let mut invalid = 0;
    for (run, error) in unavailable {
        let busy = matches!(error, inventory::RunReadError::Journal(JournalError::Busy));
        if !busy {
            invalid += 1;
        }
        runs.push(RunSummary {
            name: run.name.clone(),
            workflow: String::new(),
            kind: if busy { "active" } else { "invalid" },
            state: error.to_string(),
        });
    }
    println!("{}", serde_json::to_string(&runs)?);
    if invalid == 0 {
        Ok(())
    } else {
        Err(InspectionError::InvalidRuns { count: invalid })
    }
}

pub(crate) fn print_cells(workflow: Option<&str>, json: bool) -> Result<(), InspectionError> {
    let inventory = inventory::load()?;
    let ready: Vec<_> = inventory
        .ready
        .iter()
        .filter(|(_, inspection)| {
            workflow.is_none_or(|name| inspection.name.as_deref() == Some(name))
        })
        .collect();
    let streams: Vec<_> = inventory
        .streams
        .iter()
        .filter(|(_, inspection)| workflow.is_none_or(|name| inspection.workflow == name))
        .collect();
    if json {
        return print_cells_json(&ready, &streams, &inventory.unavailable);
    }
    if ready.is_empty() && streams.is_empty() && inventory.unavailable.is_empty() {
        match workflow {
            Some(name) => println!("no runs found for `{name}`"),
            None => println!("no runs found · run a workflow first"),
        }
        return Ok(());
    }
    println!("runs · {}", ready.len());
    for (run, inspection) in ready {
        let workflow = inspection.name.as_deref().unwrap_or("unknown workflow");
        if workflow == run.name {
            println!(
                "  {} {} · {}",
                status_marker(&inspection.status),
                run.name,
                status_summary(&inspection.status)
            );
        } else {
            println!(
                "  {} {} · {workflow} · {}",
                status_marker(&inspection.status),
                run.name,
                status_summary(&inspection.status)
            );
        }
    }
    for (run, inspection) in streams {
        println!(
            "  {} {} · {} · {}",
            stream::marker(&inspection.status),
            run.name,
            inspection.workflow,
            stream::status(&inspection.status)
        );
    }
    let result = inventory::print_unavailable(&inventory);
    if result.is_ok() {
        println!(
            "\nnext · {}",
            if inventory.ready.len() + inventory.streams.len() == 1 {
                "kairo inspect"
            } else {
                "kairo inspect <run>"
            }
        );
    }
    result
}
