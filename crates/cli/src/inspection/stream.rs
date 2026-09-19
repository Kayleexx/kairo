use kairo_runtime::{StreamRunInspection, StreamRunStatus};

pub(super) fn print(run: &str, inspection: &StreamRunInspection, verbose: bool) {
    println!("{run}");
    println!("  workflow · {}", inspection.workflow);
    println!("  state · {}", status(&inspection.status));
    println!(
        "  input · {}{}",
        inspection.input,
        inspection
            .input_source
            .as_deref()
            .filter(|source| *source != "local")
            .map_or_else(String::new, |source| format!(" · {source}"))
    );
    if !inspection.input_accepts.is_empty() {
        println!("  accepts · {}", inspection.input_accepts.join(", "));
    }
    if let Some(hash) = &inspection.input_hash {
        println!("  identity · {}", super::display_hash(hash, verbose));
    }
    if let (Some(source), Some(until)) = (&inspection.replay_source, &inspection.replay_until) {
        println!("  replay · child of {source} · through {until}");
    }
    println!("  locality · local · rerun requires this input");
    if let Some(duration) = inspection.duration_us {
        println!("  duration · {}", super::format_duration(duration));
    }
    if !inspection.values.is_empty() {
        println!(
            "  output · {}",
            inspection
                .values
                .iter()
                .map(|value| format!("{} {}", value.value, value.name))
                .collect::<Vec<_>>()
                .join(" · ")
        );
    } else if let (Some(high), Some(low)) = (inspection.high, inspection.low) {
        match (&inspection.high_label, &inspection.low_label) {
            (Some(high_label), Some(low_label)) => {
                println!("  output · {high} {high_label} · {low} {low_label}");
            }
            _ => println!("  output · {high} bytes · checksum {low:08x}"),
        }
    }
    for artifact in &inspection.outputs {
        println!("\noutput");
        println!("  {}", artifact.filename);
        println!("  {}", artifact.content_type);
        println!("  {} bytes", artifact.bytes);
        println!("  {}", super::display_hash(&artifact.hash, verbose));
        println!("  persisted · yes");
        if let Some(path) = &artifact.exported_path {
            println!("  exported · {path}");
        }
    }
    if let Some(metrics) = &inspection.metrics {
        println!("\ndata");
        println!("  streamed · {} bytes", metrics.source_bytes);
        println!("  consumed · {} bytes", metrics.consumed_bytes);
        println!("  largest batch · {} bytes", metrics.largest_batch_bytes);
        println!("  materialized · {} bytes", metrics.materialized_bytes);
        for edge in &metrics.edges {
            let Some(bytes) = edge.bytes else { continue };
            let peak = edge
                .peak_buffered_bytes
                .map_or_else(String::new, |peak| format!(" · peak buffered {peak} bytes"));
            let label = if edge.materialized == Some(true) {
                " · materialized measurement pass, not representative of direct streaming"
            } else {
                ""
            };
            println!("  edge {} · {bytes} bytes{peak}{label}", edge.name);
        }
    }
    for edge in &inspection.live_edges {
        let sent = edge
            .bytes_sent
            .map_or_else(|| "unknown".to_owned(), |bytes| format!("{bytes} bytes"));
        let received = edge
            .bytes_received
            .map_or_else(|| "unknown".to_owned(), |bytes| format!("{bytes} bytes"));
        println!("\nremote live");
        println!("  observed transport · {}", edge.transport);
        println!(
            "  workers · {} → {}",
            edge.producer_worker, edge.consumer_worker
        );
        println!("  sent · {sent} · received · {received}");
        println!("  outcome · {}", edge.outcome);
        if let Some(fallback) = &edge.fallback {
            println!("  fallback · {fallback}");
        }
    }
    println!("\ncomponents");
    for step in &inspection.steps {
        let symbol = if matches!(inspection.status, StreamRunStatus::Completed) {
            super::marker("32", "✓")
        } else {
            super::marker("36", "●")
        };
        println!("  {symbol} {step}");
    }
}

pub(super) fn status(status: &StreamRunStatus) -> &str {
    match status {
        StreamRunStatus::Running => "running",
        StreamRunStatus::Completed => "completed",
        StreamRunStatus::Failed(message) => message,
    }
}

pub(super) fn marker(status: &StreamRunStatus) -> String {
    match status {
        StreamRunStatus::Running => super::marker("36", "●"),
        StreamRunStatus::Completed => super::marker("32", "✓"),
        StreamRunStatus::Failed(_) => super::marker("31", "×"),
    }
}
