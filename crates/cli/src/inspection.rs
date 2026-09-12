use std::{collections::BTreeMap, path::Path};

use kairo_core::{Durability, Workflow, WorkflowMode};
use kairo_runtime::{
    CellInspection, CellStatus, JournalError, StreamRunStatus, inspect_cell, inspect_stream_run,
};
use thiserror::Error;

use crate::{
    discovery,
    setup::{self, SetupError},
    state::{self, LocalCell, StateError},
};

mod inventory;
mod live;
mod presentation;
mod stream;
mod wait;

use presentation::*;

#[derive(Debug, Error)]
pub(crate) enum InspectionError {
    #[error(transparent)]
    State(#[from] StateError),
    #[error("failed to inspect run `{cell}`")]
    Run {
        cell: String,
        #[source]
        source: JournalError,
    },
    #[error("{count} local run(s) could not be inspected")]
    InvalidRuns { count: usize },
    #[error("workflow `{workflow}` has multiple runs ({cells}); choose a run name")]
    AmbiguousWorkflow { workflow: String, cells: String },
    #[error(transparent)]
    Setup(#[from] SetupError),
    #[error("checkpoint `{hash}` uses `{recorded}` storage, but `{configured}` is configured")]
    BackendMismatch {
        hash: String,
        recorded: String,
        configured: &'static str,
    },
    #[error(transparent)]
    Receipt(#[from] crate::receipts::ReceiptError),
    #[error(transparent)]
    Stream(#[from] kairo_runtime::StreamRunError),
    #[error(transparent)]
    Control(#[from] kairo_control::ControlError),
    #[error(transparent)]
    WorkflowWait(#[from] kairo_runtime::WorkflowWaitError),
}

pub(crate) fn print_workflow(workflow: &Workflow, path: &Path) {
    if let Some(output) = workflow.output() {
        println!("{}\n", workflow.name());
        if let Some(description) = workflow.description() {
            println!("{description}\n");
        }
        println!("input");
        println!("  {}\n", workflow.accepts().join(" / ").to_uppercase());
        println!("output");
        println!("  {}\n", output.filename);
        println!("example");
        let example = if workflow.accepts().iter().any(|value| value.contains("mp4")) {
            "clip.mp4"
        } else {
            "notes.txt"
        };
        println!("  kairo run {} {example}", workflow.name());
        return;
    }
    let mode = match workflow.mode() {
        WorkflowMode::Scalar => "scalar",
        WorkflowMode::Stream => "stream",
    };
    println!(
        "{} · {mode} · {} components",
        workflow.name(),
        workflow.steps().len()
    );
    println!("source · {}", path.display());
    println!("\ngraph");
    for (index, step) in workflow.steps().iter().enumerate() {
        println!("  {} {}", marker("36", "●"), step.id);
        if let Some(next) = workflow.steps().get(index.saturating_add(1)) {
            let durability = match workflow.durability_after_step(index) {
                Durability::Ephemeral => "ephemeral",
                Durability::Required => "required checkpoint",
                Durability::Auto => "auto",
            };
            println!("    └─ {durability} → {}", next.id);
        }
    }
    if workflow.mode() == WorkflowMode::Stream {
        println!("\nstreaming · direct and bounded");
        println!("bytes · reported by `kairo run` for each execution");
    }
    if let Some(wait) = workflow.wait() {
        let reason = match wait {
            kairo_core::WorkflowWait::Timer(_) => "durable timer".to_owned(),
            kairo_core::WorkflowWait::Signal(name) => format!("signal {name}"),
        };
        println!(
            "\nwait · {reason} · {}",
            workflow.wait_after().map_or_else(
                || "before execution".to_owned(),
                |step| format!("after {step}")
            )
        );
    }
    if let Some(effect) = workflow.effect() {
        println!(
            "\nexternal action · {} · {}",
            effect.operation(),
            effect.after().map_or_else(
                || "after execution".to_owned(),
                |step| format!("after {step}")
            )
        );
    }
}

pub(crate) fn print_workflows(config: kairo_core::Config) -> Result<(), InspectionError> {
    discovery::print_references(config);
    let inventory = inventory::load()?;
    let mut workflows = BTreeMap::<String, (usize, usize)>::new();
    for (_, inspection) in &inventory.ready {
        let name = inspection.name.as_deref().unwrap_or("unknown").to_owned();
        let entry = workflows.entry(name).or_default();
        entry.0 += 1;
        entry.1 += usize::from(matches!(inspection.status, CellStatus::Completed { .. }));
    }
    for (_, inspection) in &inventory.streams {
        let entry = workflows.entry(inspection.workflow.clone()).or_default();
        entry.0 += 1;
        entry.1 += usize::from(matches!(inspection.status, StreamRunStatus::Completed));
    }
    if workflows.is_empty() {
        println!("no workflows observed · run a workflow first");
    } else {
        println!("workflows · {}", workflows.len());
        for (name, (cells, completed)) in workflows {
            println!("  {name} · {cells} runs · {completed} completed");
        }
        println!("\nRuns · local history");
    }
    inventory::print_unavailable(&inventory)
}

pub(crate) fn print_cells(workflow: Option<&str>) -> Result<(), InspectionError> {
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
    if ready.is_empty() && streams.is_empty() && inventory.unavailable.is_empty() {
        match workflow {
            Some(name) => println!("no runs found for `{name}`"),
            None => println!("no runs found · run a workflow first"),
        }
        return Ok(());
    }
    println!("runs · {}", ready.len());
    for (cell, inspection) in ready {
        let workflow = inspection.name.as_deref().unwrap_or("unknown workflow");
        if workflow == cell.name {
            println!(
                "  {} {} · {}",
                status_marker(&inspection.status),
                cell.name,
                status_summary(&inspection.status)
            );
        } else {
            println!(
                "  {} {} · {workflow} · {}",
                status_marker(&inspection.status),
                cell.name,
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

pub(crate) async fn print_cell(
    requested: Option<&Path>,
    verify: bool,
    verbose: bool,
) -> Result<(), InspectionError> {
    let cell = select_cell(requested)?;
    let live_status = live::status(&cell.name)?;
    if !cell.path.exists()
        && let Some(status) = live_status
    {
        live::print(&cell.name, status);
        return Ok(());
    }
    if cell.path.exists()
        && let Some(inspection) = inspect_stream_run(&cell.path)?
    {
        stream::print(&cell.name, &inspection, verbose);
        return Ok(());
    }
    let inspection = inspect(&cell.path)?;
    let name = inspection.name.as_deref().unwrap_or(&cell.name);
    println!("{}", cell.name);
    println!("  workflow · {name}");
    if !live_status
        .as_ref()
        .is_some_and(|status| live::print_state(&cell.name, status))
    {
        println!("  state · {}", status_detail(&inspection.status));
    }
    println!("  input · {}", inspection.input);
    if let CellStatus::Completed { output } = inspection.status {
        println!("  output · {output}");
    }
    println!("  duration · {}", total_duration(&inspection));
    if !inspection.metadata_complete {
        println!("  metadata · partial · recorded by an older Kairo version");
    }
    println!("\ncomponents");
    for component in &inspection.components {
        let output = component
            .output
            .map_or_else(|| "pending".to_owned(), |output| output.to_string());
        let duration = component
            .duration_us
            .map(format_duration)
            .map_or_else(String::new, |duration| format!(" · {duration}"));
        println!(
            "  {} {} · {} → {}{}",
            if component.output.is_some() {
                marker("32", "✓")
            } else {
                marker("33", "!")
            },
            component.name,
            component.input,
            output,
            duration
        );
        println!("    component · {}", display_hash(&component.hash, verbose));
        if component.attempts > 1 {
            println!("    attempts · {}", component.attempts);
        }
        match (&component.checkpoint, component.durable_after) {
            (Some(hash), _) => {
                println!("    checkpoint · {}", display_hash(hash, verbose));
                if let Some(backend) = &component.checkpoint_backend {
                    println!("    storage · {backend}");
                }
            }
            (None, Some(true)) => println!("    checkpoint · pending"),
            (None, Some(false)) => println!("    edge · ephemeral"),
            (None, None) => {}
        }
    }
    wait::print(&cell.path)?;
    crate::receipts::print(&cell.path, verbose)?;
    if verify {
        verify_checkpoints(&inspection).await?;
    }
    Ok(())
}

fn select_cell(requested: Option<&Path>) -> Result<LocalCell, InspectionError> {
    let Some(requested) = requested else {
        return state::select(None).map_err(Into::into);
    };
    let exact = state::select(Some(requested))?;
    if exact.path.exists() || requested.components().count() != 1 || requested.extension().is_some()
    {
        return Ok(exact);
    }
    let workflow = requested.to_string_lossy();
    let inventory = inventory::load()?;
    let mut matches: Vec<_> = inventory
        .ready
        .into_iter()
        .filter(|(_, inspection)| inspection.name.as_deref() == Some(workflow.as_ref()))
        .map(|(cell, _)| cell)
        .collect();
    matches.extend(
        inventory
            .streams
            .into_iter()
            .filter(|(_, inspection)| inspection.workflow == workflow.as_ref())
            .map(|(cell, _)| cell),
    );
    match matches.len() {
        0 => Ok(exact),
        1 => matches.into_iter().next().ok_or(StateError::NoCells.into()),
        _ => Err(InspectionError::AmbiguousWorkflow {
            workflow: workflow.into_owned(),
            cells: matches
                .iter()
                .map(|cell| cell.name.as_str())
                .collect::<Vec<_>>()
                .join(", "),
        }),
    }
}

async fn verify_checkpoints(inspection: &CellInspection) -> Result<(), InspectionError> {
    let checkpoints: Vec<_> = inspection
        .components
        .iter()
        .filter_map(|component| {
            component
                .checkpoint
                .as_deref()
                .map(|hash| (hash, component.checkpoint_backend.as_deref()))
        })
        .collect();
    if checkpoints.is_empty() {
        println!("\nverification · no checkpoints recorded");
        return Ok(());
    }
    let store = setup::artifact_store()?;
    let configured = store.backend().as_str();
    for (hash, recorded) in &checkpoints {
        if let Some(recorded) = recorded.filter(|recorded| *recorded != configured) {
            return Err(InspectionError::BackendMismatch {
                hash: (*hash).to_owned(),
                recorded: (*recorded).to_owned(),
                configured,
            });
        }
        store.get(hash).await.map_err(SetupError::Storage)?;
    }
    println!(
        "\nverification · {} checkpoint(s) available in {configured} storage",
        checkpoints.len()
    );
    Ok(())
}

fn inspect(path: &Path) -> Result<CellInspection, InspectionError> {
    inspect_cell(path).map_err(|source| InspectionError::Run {
        cell: path.display().to_string(),
        source,
    })
}
