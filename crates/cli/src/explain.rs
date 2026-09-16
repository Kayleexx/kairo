use std::path::Path;

use kairo_runtime::inspect_stream_run;
use kairo_tui::explain::{
    Cost, Durability, ExplainEntry, Metric, Placement, Transport, explain_cell, explain_value_cell,
};

use crate::inspection::{self, InspectionError, inspect_aggregated_value, select_cell};

pub(crate) async fn print(requested: Option<&Path>, json: bool) -> crate::Result<()> {
    let cell = select_cell(requested)?;
    if inspect_stream_run(&cell.path)
        .map_err(InspectionError::from)?
        .is_some()
    {
        return Err(InspectionError::Unexplainable {
            cell: cell.name,
            kind: "stream",
        }
        .into());
    }
    let value = inspect_aggregated_value(&cell.path).map_err(|source| InspectionError::Run {
        cell: cell.name.clone(),
        source,
    })?;
    let entries = if let Some(value) = value {
        explain_value_cell(&value)
    } else {
        let inspection = inspection::inspect(&cell.path)?;
        let history = inspection::assignment_history(&cell.name)
            .map_err(InspectionError::from)?
            .unwrap_or_default();
        explain_cell(&inspection, &history)
    };

    if json {
        let text = serde_json::to_string_pretty(&entries).map_err(InspectionError::from)?;
        println!("{text}");
        return Ok(());
    }
    println!("explain · {}", cell.name);
    if entries.is_empty() {
        println!("  (single-step workflow -- no edges to explain)");
    }
    for entry in &entries {
        print_entry(entry);
    }
    Ok(())
}

fn print_entry(entry: &ExplainEntry) {
    println!("  step · {}", entry.step);
    println!("    placement   {}", describe_placement(&entry.placement));
    println!("    transport   {}", describe_transport(entry.transport));
    println!("    durability  {}", describe_durability(&entry.durability));
    print_cost(&entry.cost);
    if let Some(reason) = &entry.durability_reason {
        println!("    reason      {reason}");
    }
    if let Some(reason) = &entry.placement_reason {
        println!("    moved       {reason}");
    }
}

fn describe_placement(placement: &Placement) -> String {
    match placement {
        Placement::Same { worker } => format!("same worker · {worker}"),
        Placement::Moved {
            from,
            to,
            target_had_cache,
        } => {
            let cache = match target_had_cache {
                Some(true) => " · target had cache",
                Some(false) => " · target had no cache",
                None => "",
            };
            format!("{from} -> {to}{cache}")
        }
        Placement::NotDistributed => "n/a -- value workflows always run in-process".to_owned(),
        Placement::Unknown => "unknown -- no assignment history recorded".to_owned(),
    }
}

fn describe_transport(transport: Transport) -> &'static str {
    match transport {
        Transport::InProcess => "in-process",
        Transport::DurableArtifactHandoff => "durable artifact handoff",
    }
}

fn describe_durability(durability: &Durability) -> String {
    match durability {
        Durability::Declared { required: true } => "required (declared)".to_owned(),
        Durability::Declared { required: false } => "ephemeral (declared)".to_owned(),
        Durability::AutoResolved {
            required,
            profile_id,
        } => format!(
            "{} (auto-resolved, profile {profile_id})",
            if *required { "required" } else { "ephemeral" }
        ),
    }
}

fn print_cost(cost: &Cost) {
    println!("    cost");
    println!(
        "      recompute   {}",
        describe_metric(cost.recompute_us, "us")
    );
    println!(
        "      checkpoint  {}",
        describe_metric(cost.checkpoint_us, "us")
    );
    println!(
        "      size        {}",
        describe_metric(cost.checkpoint_bytes, "bytes")
    );
}

fn describe_metric(metric: Metric, unit: &str) -> String {
    match metric {
        Metric::Measured(value) => format!("{value} {unit} (measured)"),
        Metric::Estimated(value) => format!("~{value} {unit} (estimated, from prior profile)"),
        Metric::Unknown => "unknown".to_owned(),
    }
}
