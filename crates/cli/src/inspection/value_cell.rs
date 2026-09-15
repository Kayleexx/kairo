use kairo_runtime::{ValueRunInspection, ValueRunStatus};

use super::presentation::{display_hash, format_duration, marker};

fn status_detail(status: &ValueRunStatus) -> String {
    match status {
        ValueRunStatus::Completed { .. } => "completed".to_owned(),
        ValueRunStatus::Ready { next_index } => format!("ready for component {}", next_index + 1),
        ValueRunStatus::Interrupted { step } => format!("recoverable · interrupted at {step}"),
        ValueRunStatus::CheckpointPending { step } => {
            format!("recoverable · checkpoint pending after {step}")
        }
        ValueRunStatus::Finalizing => "recoverable · finalizing".to_owned(),
    }
}

fn total_duration(inspection: &ValueRunInspection) -> String {
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

pub(super) fn print(run: &str, inspection: &ValueRunInspection, verbose: bool) {
    println!("{run}");
    println!("  workflow · {}", inspection.name.as_deref().unwrap_or(run));
    println!("  state · {}", status_detail(&inspection.status));
    println!("  input · {}", inspection.input_preview);
    if let ValueRunStatus::Completed { output_preview } = &inspection.status {
        println!("  output · {output_preview}");
    }
    println!("  duration · {}", total_duration(inspection));
    if let Some(recovery) = inspection.recovery_duration_us {
        println!("  recovery · {}", format_duration(recovery));
    }
    if !inspection.metadata_complete {
        println!("  metadata · partial · recorded by an older Kairo version");
    }
    println!("\ncomponents");
    for component in &inspection.components {
        let output = component
            .output_preview
            .clone()
            .unwrap_or_else(|| "pending".to_owned());
        let duration = component
            .duration_us
            .map(format_duration)
            .map_or_else(String::new, |duration| format!(" · {duration}"));
        println!(
            "  {} {} · {} → {}{}",
            if component.output_preview.is_some() {
                marker("32", "✓")
            } else {
                marker("33", "!")
            },
            component.name,
            component.input_preview,
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
            println!("    auto · {reason}");
        }
    }
}
