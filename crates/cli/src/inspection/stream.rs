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
    println!("  locality · local · rerun requires this input");
    if let Some(duration) = inspection.duration_us {
        println!("  duration · {}", super::format_duration(duration));
    }
    if let (Some(high), Some(low)) = (inspection.high, inspection.low) {
        match (&inspection.high_label, &inspection.low_label) {
            (Some(high_label), Some(low_label)) => {
                println!("  output · {high} {high_label} · {low} {low_label}");
            }
            _ => println!("  output · {high} bytes · checksum {low:08x}"),
        }
    }
    if let Some(metrics) = inspection.metrics {
        println!("\ndata");
        println!("  streamed · {} bytes", metrics.source_bytes);
        println!("  consumed · {} bytes", metrics.consumed_bytes);
        println!("  largest batch · {} bytes", metrics.largest_batch_bytes);
        println!("  materialized · {} bytes", metrics.materialized_bytes);
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
