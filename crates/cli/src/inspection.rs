use std::{collections::BTreeMap, path::Path};

use kairo_core::{Durability, IoInput, IoOutput, Workflow, WorkflowMode};
use kairo_runtime::{CellStatus, JournalError, Runtime, StreamRunStatus, ValueRunStatus};
use thiserror::Error;

use crate::{discovery, setup::SetupError, state::StateError};

mod cells;
mod detail;
mod groups;
mod inventory;
mod live;
mod presentation;
mod prune;
mod resume;
mod stream;
mod value_cell;
mod wait;

pub(crate) use cells::print_cells;
pub(crate) use detail::{inspect, inspect_value, print_cell, select_cell};
pub(crate) use groups::{inspect_aggregated, inspect_aggregated_value, sibling_groups};
pub(crate) use live::assignment_history;
pub(crate) use presentation::total_duration_us;
use presentation::*;
pub(crate) use prune::{PruneOptions, prune};
pub(crate) use resume::resume;

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
    #[error("failed to remove run files at `{path}`")]
    Prune {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to encode JSON output")]
    Json(#[from] serde_json::Error),
}

pub(crate) fn print_workflow(runtime: &Runtime, workflow: &Workflow, path: &Path) {
    let mode = match workflow.mode() {
        WorkflowMode::Scalar => "scalar",
        WorkflowMode::Stream => "stream",
        WorkflowMode::Value => "value",
    };
    println!(
        "{} · {mode} · {} components",
        workflow.name(),
        workflow.steps().len()
    );
    if let Some(description) = workflow.description() {
        println!("{description}");
    }
    println!("input · {}", input_summary(workflow));
    println!("output · {}", output_summary(workflow));
    println!("source · {}", path.display());
    println!("\ngraph");
    for (index, step) in workflow.steps().iter().enumerate() {
        println!("  {} {}", marker("36", "●"), step.id);
        if let Some(next) = workflow.steps().get(index.saturating_add(1)) {
            let auto_status;
            let durability = match workflow.durability_after_step(index) {
                Durability::Ephemeral => "ephemeral",
                Durability::Required => "required checkpoint",
                Durability::Auto => {
                    auto_status = match runtime.auto_edge_profile(workflow, index) {
                        Ok(Some(status)) => format!("auto ({status})"),
                        Ok(None) => format!(
                            "auto (no profile yet · run `kairo workflow profile {}`)",
                            workflow.name()
                        ),
                        Err(_) => "auto (unresolved)".to_owned(),
                    };
                    &auto_status
                }
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

fn input_summary(workflow: &Workflow) -> String {
    let kind = match workflow.io().input {
        IoInput::None => "none",
        IoInput::File => "file",
        IoInput::Value => "value",
    };
    if workflow.accepts().is_empty() {
        kind.to_owned()
    } else {
        format!("{kind} · {}", workflow.accepts().join(" / "))
    }
}

fn output_summary(workflow: &Workflow) -> String {
    if let Some(output) = workflow.output() {
        return output.filename.clone();
    }
    match workflow.io().output {
        IoOutput::None => "run result".to_owned(),
        IoOutput::Value => "value".to_owned(),
        IoOutput::Artifact => "artifact".to_owned(),
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
    for (_, inspection) in &inventory.values {
        let name = inspection.name.as_deref().unwrap_or("unknown").to_owned();
        let entry = workflows.entry(name).or_default();
        entry.0 += 1;
        entry.1 += usize::from(matches!(
            inspection.status,
            ValueRunStatus::Completed { .. }
        ));
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
