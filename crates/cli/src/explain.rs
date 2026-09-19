use std::path::Path;

use kairo_runtime::inspect_stream_run;
use kairo_tui::explain::{
    Cost, Durability, ExplainEntry, Metric, Placement, Transport, explain_cell, explain_value_cell,
};
use serde::Serialize;

use crate::inspection::{self, InspectionError, inspect_aggregated_value, select_cell};

pub(crate) async fn print(requested: Option<&Path>, json: bool) -> crate::Result<()> {
    let cell = select_cell(requested)?;
    if let Some(stream) = inspect_stream_run(&cell.path).map_err(InspectionError::from)? {
        return print_stream(&cell.name, &stream, json);
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

#[derive(Serialize)]
struct StreamExplain<'a> {
    planned_transport: &'static str,
    observed_transport: &'a str,
    producer_worker: &'a str,
    consumer_worker: &'a str,
    bytes_sent: Option<u64>,
    bytes_received: Option<u64>,
    outcome: &'a str,
    fallback: Option<&'a str>,
    replay_source: Option<&'a str>,
    replay_until: Option<&'a str>,
}

fn print_stream(
    name: &str,
    stream: &kairo_runtime::StreamRunInspection,
    json: bool,
) -> crate::Result<()> {
    let entries: Vec<_> = stream
        .live_edges
        .iter()
        .map(|edge| StreamExplain {
            // StreamRun stores observations, not a planner prediction. Do not turn a successful
            // observation into retroactive planner intent.
            planned_transport: "not recorded",
            observed_transport: &edge.transport,
            producer_worker: &edge.producer_worker,
            consumer_worker: &edge.consumer_worker,
            bytes_sent: edge.bytes_sent,
            bytes_received: edge.bytes_received,
            outcome: &edge.outcome,
            fallback: edge.fallback.as_deref(),
            replay_source: stream.replay_source.as_deref(),
            replay_until: stream.replay_until.as_deref(),
        })
        .collect();
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&entries).map_err(InspectionError::from)?
        );
        return Ok(());
    }
    println!("explain · {name}");
    if let (Some(source), Some(until)) = (&stream.replay_source, &stream.replay_until) {
        println!("  replay source       {source}");
        println!("  replay through      {until}");
    }
    if entries.is_empty() {
        println!("  no observed physical stream transport");
    }
    for edge in entries {
        println!("  planned transport   {}", edge.planned_transport);
        println!("  observed transport  {}", edge.observed_transport);
        println!(
            "  workers             {} → {}",
            edge.producer_worker, edge.consumer_worker
        );
        println!(
            "  bytes               sent {:?} · received {:?}",
            edge.bytes_sent, edge.bytes_received
        );
        println!("  outcome             {}", edge.outcome);
        if let Some(fallback) = edge.fallback {
            println!("  fallback            {fallback}");
        }
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
    if cost.samples.is_some() || cost.age_ms.is_some() {
        println!(
            "      profile     {}, {}",
            cost.samples.map_or_else(
                || "unknown sample count".to_owned(),
                |samples| format!("{samples} sample(s)")
            ),
            cost.age_ms.map_or_else(
                || "unknown age".to_owned(),
                |age_ms| format!("{} old", format_age(age_ms))
            )
        );
    }
}

fn format_age(age_ms: u64) -> String {
    let seconds = age_ms / 1000;
    if seconds < 60 {
        return format!("{seconds}s");
    }
    let minutes = seconds / 60;
    if minutes < 60 {
        return format!("{minutes}m");
    }
    let hours = minutes / 60;
    if hours < 24 {
        return format!("{hours}h");
    }
    format!("{}d", hours / 24)
}

fn describe_metric(metric: Metric, unit: &str) -> String {
    match metric {
        Metric::Measured(value) => format!("{value} {unit} (measured)"),
        Metric::Estimated(value) => format!("~{value} {unit} (estimated, from prior profile)"),
        Metric::Unknown => "unknown".to_owned(),
    }
}
