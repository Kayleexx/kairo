use std::path::Path;

use kairo_runtime::{CellInspection, CellStatus, inspect_stream_run};

use crate::{
    setup::{self, SetupError},
    state::{self, LocalCell, StateError},
};

use super::{
    InspectionError, display_hash, format_duration, groups::inspect_aggregated,
    groups::inspect_aggregated_value, inventory, live, marker, status_detail, stream,
    total_duration, value_cell, wait,
};

pub(crate) async fn print_cell(
    requested: Option<&Path>,
    verify: bool,
    verbose: bool,
    export: Option<&Path>,
) -> crate::Result<()> {
    let cell = select_cell(requested)?;
    let live_status = live::status(&cell.name)?;
    if !cell.path.exists()
        && let Some(status) = live_status
    {
        live::print(&cell.name, status);
        return if export.is_some() {
            Err(crate::CliError::Output)
        } else {
            Ok(())
        };
    }
    if cell.path.exists()
        && let Some(inspection) = inspect_stream_run(&cell.path)?
    {
        stream::print(&cell.name, &inspection, verbose);
        if let Some(destination) = export {
            let artifact = inspection.outputs.first().ok_or(crate::CliError::Output)?;
            let store = setup::artifact_store()?;
            crate::stream::export(&store, &artifact.hash, destination).await?;
            println!("\nexported · {}", destination.display());
        }
        return Ok(());
    }
    if cell.path.exists()
        && let Some(inspection) = inspect_value(&cell.path)?
    {
        value_cell::print(&cell.name, &inspection, verbose);
        return if export.is_some() {
            Err(crate::CliError::Output)
        } else {
            Ok(())
        };
    }
    if export.is_some() {
        return Err(crate::CliError::Output);
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
    if let Some(recovery) = inspection.recovery_duration_us {
        println!("  recovery · {}", format_duration(recovery));
    }
    // incomplete metadata is expected and already explained by "state" for a run that hasn't
    // finished -- only worth a separate note when a *completed* run still has old-format gaps.
    if !inspection.metadata_complete && matches!(inspection.status, CellStatus::Completed { .. }) {
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
                if let Some(bytes) = component.checkpoint_bytes {
                    println!("    checkpoint size · {bytes} bytes");
                }
                if let Some(duration_us) = component.checkpoint_duration_us {
                    println!("    checkpoint time · {}", format_duration(duration_us));
                }
            }
            (None, Some(true)) => println!("    checkpoint · pending"),
            (None, Some(false)) => println!("    edge · ephemeral"),
            (None, None) => {}
        }
        if let Some(reason) = &component.durability_reason {
            println!("    auto · {}", super::compact_reason(reason, verbose));
        }
    }
    if let Some(history) = live::assignment_history(&cell.name)?
        && history
            .iter()
            .filter(|event| matches!(event, kairo_control::RunEvent::Assigned { .. }))
            .count()
            > 1
    {
        if verbose {
            live::print_placement(&history);
        } else if let Some(moves) = live::moved_summary(&history) {
            println!("\n{moves}");
        }
    }
    wait::print(&cell.path).map_err(InspectionError::from)?;
    crate::receipts::print(&cell.path, verbose).map_err(InspectionError::from)?;
    if verify {
        verify_checkpoints(&inspection).await?;
    }
    Ok(())
}

pub(crate) fn select_cell(requested: Option<&Path>) -> Result<LocalCell, InspectionError> {
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

pub(crate) fn inspect_value(
    path: &Path,
) -> Result<Option<kairo_runtime::ValueRunInspection>, InspectionError> {
    inspect_aggregated_value(path).map_err(|source| InspectionError::Run {
        cell: path.display().to_string(),
        source,
    })
}

pub(crate) fn inspect(path: &Path) -> Result<CellInspection, InspectionError> {
    inspect_aggregated(path).map_err(|source| InspectionError::Run {
        cell: path.display().to_string(),
        source,
    })
}
