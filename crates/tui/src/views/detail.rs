use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    widgets::{Paragraph, Widget, Wrap},
};

use crate::{App, activity};

use super::{empty, panel};

pub(super) fn draw(area: Rect, buffer: &mut Buffer, app: &App) {
    let Some(run) = app.runs.get(app.selected) else {
        return empty(area, buffer, "No runs yet. Run a workflow first.");
    };
    let mut lines = vec![format!(
        "{}\n{}\nrun id · {}",
        super::label(run),
        activity(run),
        run.name
    )];
    if let Some(kairo_control::RunStatus::Completed { output, worker }) = &run.service {
        if !worker.is_empty() {
            lines.push(format!("worker · {worker}"));
        }
        // the inspection/stream/value branches below already show a richer, mode-specific
        // output line -- this generic one only fills in when none of them apply.
        if run.inspection.is_none() && run.stream.is_none() && run.value.is_none() {
            lines.push(format!("output · {output}"));
        }
    }
    if let Some(inspection) = &run.inspection {
        lines.push(format!("input · {}", inspection.input));
        let explained = crate::explain::explain_cell(inspection, &run.history);
        for (index, component) in inspection.components.iter().enumerate() {
            if component.index > 0 {
                lines.push("  ↓".to_owned());
            }
            let state = component.output.map_or_else(
                || "running".to_owned(),
                |output| format!("completed · output {output}"),
            );
            let duration = component
                .duration_us
                .map(format_duration)
                .map_or_else(String::new, |value| format!(" · {value}"));
            lines.push(format!("{} · {state}{duration}", component.name));
            if let Some(true) = component.durable_after {
                lines.push(match &component.checkpoint {
                    Some(_) => "  └─ checkpoint saved".to_owned(),
                    None => "  └─ checkpoint pending".to_owned(),
                });
            }
            if let Some(entry) = explained.get(index) {
                if let Some(reason) = &entry.durability_reason {
                    lines.push(format!("  └─ auto · {reason}"));
                }
                if let Some(reason) = &entry.placement_reason {
                    lines.push(format!("  └─ moved · {reason}"));
                }
            }
        }
        if let Some(wait) = &run.wait {
            let reason = match &wait.wait {
                kairo_runtime::DurableWait::Signal { name } => format!("signal {name}"),
                kairo_runtime::DurableWait::Timer { .. } => "durable timer".to_owned(),
            };
            lines.push(format!(
                "wait · {reason} · {}",
                if wait.completed { "resumed" } else { "waiting" }
            ));
        }
        if let Some(path) = &run.path
            && let Ok(receipts) = kairo_runtime::inspect_receipts(path)
        {
            for receipt in receipts {
                lines.push(format!(
                    "external action · {} · {}{}",
                    receipt.operation,
                    receipt.status,
                    if receipt.reused { " · reused" } else { "" }
                ));
            }
        }
    } else if let Some(stream) = &run.stream {
        lines.push(format!(
            "input · {}{}",
            stream.input,
            stream
                .input_source
                .as_deref()
                .filter(|source| *source != "local")
                .map_or_else(String::new, |source| format!(" · {source}"))
        ));
        if !stream.input_accepts.is_empty() {
            lines.push(format!("accepts · {}", stream.input_accepts.join(", ")));
        }
        if let Some(hash) = &stream.input_hash {
            lines.push(format!("identity · {}", short_hash(hash)));
        }
        if let (Some(source), Some(until)) = (&stream.replay_source, &stream.replay_until) {
            lines.push(format!("replay · child of {source} · through {until}"));
        }
        if let Some(boundary) = &stream.replay_boundary {
            lines.push(format!("replay boundary · after {boundary}"));
        }
        lines.push("local input · rerun requires this file".to_owned());
        if !stream.values.is_empty() {
            lines.push(format!(
                "output · {}",
                stream
                    .values
                    .iter()
                    .map(|value| format!("{} {}", value.value, value.name))
                    .collect::<Vec<_>>()
                    .join(" · ")
            ));
        } else if let (Some(high), Some(low)) = (stream.high, stream.low) {
            lines.push(match (&stream.high_label, &stream.low_label) {
                (Some(high_label), Some(low_label)) => {
                    format!("output · {high} {high_label} · {low} {low_label}")
                }
                _ => format!("output · {high} bytes · checksum {low:08x}"),
            });
        }
        for artifact in &stream.outputs {
            lines.push(format!(
                "output · {}\n{}\n{} bytes\n{}\npersisted · yes{}",
                artifact.filename,
                artifact.content_type,
                artifact.bytes,
                short_hash(&artifact.hash),
                artifact
                    .exported_path
                    .as_ref()
                    .map_or_else(String::new, |path| format!("\nexported · {path}"))
            ));
        }
        if let Some(metrics) = &stream.metrics {
            lines.push(format!(
                "streamed · {} bytes\nconsumed · {} bytes\nlargest batch · {} bytes\nmaterialized · {} bytes",
                metrics.source_bytes,
                metrics.consumed_bytes,
                metrics.largest_batch_bytes,
                metrics.materialized_bytes
            ));
        }
        lines.extend(super::stream::live_edges(&stream.live_edges));
        for step in &stream.steps {
            lines.push(format!("{step} · {}", activity(run)));
        }
    } else if let Some(value) = &run.value {
        lines.push(format!("input · {}", value.input_preview));
        if let kairo_runtime::ValueRunStatus::Completed { output_preview } = &value.status {
            lines.push(format!("output · {output_preview}"));
        }
        for component in &value.components {
            if component.index > 0 {
                lines.push("  ↓".to_owned());
            }
            let state = component
                .output_preview
                .clone()
                .unwrap_or_else(|| "running".to_owned());
            let duration = component
                .duration_us
                .map(format_duration)
                .map_or_else(String::new, |value| format!(" · {value}"));
            lines.push(format!(
                "{} · {} → {state}{duration}",
                component.name, component.input_preview
            ));
            if let Some(true) = component.durable_after {
                lines.push(match &component.checkpoint {
                    Some(_) => "  └─ checkpoint saved".to_owned(),
                    None => "  └─ checkpoint pending".to_owned(),
                });
            }
        }
    } else if run.error.is_none() {
        lines.push("Waiting for local progress to be recorded.".to_owned());
    }
    Paragraph::new(lines.join("\n"))
        .style(Style::default().fg(Color::White).bg(Color::Black))
        .block(panel("Run detail"))
        .wrap(Wrap { trim: true })
        .render(area, buffer);
}

fn short_hash(hash: &str) -> &str {
    hash.get(..19).unwrap_or(hash)
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
