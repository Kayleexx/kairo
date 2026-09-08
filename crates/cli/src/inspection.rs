use std::{
    collections::BTreeMap,
    io::{self, IsTerminal},
    path::Path,
};

use kairo_core::{Durability, Workflow, WorkflowMode};
use kairo_runtime::{CellInspection, CellStatus, JournalError, inspect_cell};
use thiserror::Error;

use crate::{
    setup::{self, SetupError},
    state::{self, LocalCell, StateError},
};

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
}

pub(crate) fn print_workflow(workflow: &Workflow, path: &Path) {
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
            };
            println!("    └─ {durability} → {}", next.id);
        }
    }
    if workflow.mode() == WorkflowMode::Stream {
        println!("\nstreaming · direct and bounded");
        println!("bytes · reported by `kairo run` for each execution");
    }
}

pub(crate) fn print_workflows() -> Result<(), InspectionError> {
    let inventory = inventory()?;
    let mut workflows = BTreeMap::<String, (usize, usize)>::new();
    for (_, inspection) in &inventory.ready {
        let name = inspection.name.as_deref().unwrap_or("unknown").to_owned();
        let entry = workflows.entry(name).or_default();
        entry.0 += 1;
        entry.1 += usize::from(matches!(inspection.status, CellStatus::Completed { .. }));
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
    print_unavailable(&inventory)
}

pub(crate) fn print_cells(workflow: Option<&str>) -> Result<(), InspectionError> {
    let inventory = inventory()?;
    let ready: Vec<_> = inventory
        .ready
        .iter()
        .filter(|(_, inspection)| {
            workflow.is_none_or(|name| inspection.name.as_deref() == Some(name))
        })
        .collect();
    if ready.is_empty() && inventory.unavailable.is_empty() {
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
    let result = print_unavailable(&inventory);
    if result.is_ok() {
        println!(
            "\nnext · {}",
            if inventory.ready.len() == 1 {
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
    let inspection = inspect(&cell.path)?;
    let name = inspection.name.as_deref().unwrap_or(&cell.name);
    println!("{}", cell.name);
    println!("  workflow · {name}");
    println!("  state · {}", status_detail(&inspection.status));
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
    if verify {
        verify_checkpoints(&inspection).await?;
    }
    Ok(())
}

struct Inventory {
    ready: Vec<(LocalCell, CellInspection)>,
    unavailable: Vec<(LocalCell, JournalError)>,
}

fn inventory() -> Result<Inventory, StateError> {
    let mut inventory = Inventory {
        ready: Vec::new(),
        unavailable: Vec::new(),
    };
    for cell in state::discover()? {
        match inspect_cell(&cell.path) {
            Ok(inspection) => inventory.ready.push((cell, inspection)),
            Err(error) => inventory.unavailable.push((cell, error)),
        }
    }
    Ok(inventory)
}

fn print_unavailable(inventory: &Inventory) -> Result<(), InspectionError> {
    for (cell, error) in &inventory.unavailable {
        let (symbol, state) = if matches!(error, JournalError::Busy) {
            (marker("36", "●"), "active")
        } else {
            (marker("31", "×"), "invalid")
        };
        println!("  {symbol} {} · {state} · {error}", cell.name);
    }
    let invalid = inventory
        .unavailable
        .iter()
        .filter(|(_, error)| !matches!(error, JournalError::Busy))
        .count();
    if invalid > 0 {
        Err(InspectionError::InvalidRuns { count: invalid })
    } else {
        Ok(())
    }
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
    let matches: Vec<_> = inventory()?
        .ready
        .into_iter()
        .filter(|(_, inspection)| inspection.name.as_deref() == Some(workflow.as_ref()))
        .map(|(cell, _)| cell)
        .collect();
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

fn status_marker(status: &CellStatus) -> String {
    match status {
        CellStatus::Completed { .. } => marker("32", "✓"),
        CellStatus::Ready { .. } => marker("36", "●"),
        CellStatus::Interrupted { .. } | CellStatus::CheckpointPending { .. } => marker("33", "!"),
        CellStatus::Finalizing => marker("36", "●"),
    }
}

fn status_summary(status: &CellStatus) -> String {
    match status {
        CellStatus::Completed { output } => format!("completed · output {output}"),
        CellStatus::Ready { next_index } => format!("ready · next component {}", next_index + 1),
        CellStatus::Interrupted { step } => format!("recoverable · interrupted at {step}"),
        CellStatus::CheckpointPending { step } => {
            format!("recoverable · checkpoint pending after {step}")
        }
        CellStatus::Finalizing => "recoverable · finalizing".to_owned(),
    }
}

fn status_detail(status: &CellStatus) -> String {
    match status {
        CellStatus::Completed { .. } => "completed".to_owned(),
        CellStatus::Ready { next_index } => format!("ready for component {}", next_index + 1),
        CellStatus::Interrupted { step } => format!("recoverable · interrupted at {step}"),
        CellStatus::CheckpointPending { step } => {
            format!("recoverable · checkpoint pending after {step}")
        }
        CellStatus::Finalizing => "recoverable · finalizing".to_owned(),
    }
}

fn total_duration(inspection: &CellInspection) -> String {
    inspection
        .components
        .iter()
        .try_fold(0_u64, |total, component| {
            component
                .duration_us
                .and_then(|duration| total.checked_add(duration))
        })
        .map_or_else(|| "unavailable".to_owned(), format_duration)
}

fn format_duration(microseconds: u64) -> String {
    if microseconds >= 1_000_000 {
        format!(
            "{}.{:03}s",
            microseconds / 1_000_000,
            microseconds % 1_000_000 / 1_000
        )
    } else if microseconds >= 1_000 {
        format!("{}.{:03}ms", microseconds / 1_000, microseconds % 1_000)
    } else {
        format!("{microseconds}µs")
    }
}

fn display_hash(hash: &str, verbose: bool) -> String {
    if verbose || !hash.is_ascii() || hash.len() <= 27 {
        return hash.to_owned();
    }
    format!("{}…{}", &hash[..15], &hash[hash.len() - 8..])
}

fn marker(color: &str, symbol: &str) -> String {
    if crate::color_enabled(io::stdout().is_terminal()) {
        format!("\x1b[{color}m{symbol}\x1b[0m")
    } else {
        symbol.to_owned()
    }
}
