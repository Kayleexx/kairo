use std::io::{self, IsTerminal};

use kairo_runtime::{CellInspection, CellStatus, ValueRunStatus};

pub(super) fn status_marker(status: &CellStatus) -> String {
    match status {
        CellStatus::Completed { .. } => marker("32", "✓"),
        CellStatus::Ready { .. } => marker("36", "●"),
        CellStatus::Interrupted { .. } | CellStatus::CheckpointPending { .. } => marker("33", "!"),
        CellStatus::Finalizing => marker("36", "●"),
    }
}

pub(super) fn status_summary(status: &CellStatus) -> String {
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

pub(super) fn status_detail(status: &CellStatus) -> String {
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

pub(super) fn value_status_marker(status: &ValueRunStatus) -> String {
    match status {
        ValueRunStatus::Completed { .. } => marker("32", "✓"),
        ValueRunStatus::Ready { .. } => marker("36", "●"),
        ValueRunStatus::Interrupted { .. } | ValueRunStatus::CheckpointPending { .. } => {
            marker("33", "!")
        }
        ValueRunStatus::Finalizing => marker("36", "●"),
    }
}

pub(super) fn value_status_summary(status: &ValueRunStatus) -> String {
    match status {
        ValueRunStatus::Completed { output_preview } => {
            format!("completed · output {output_preview}")
        }
        ValueRunStatus::Ready { next_index } => {
            format!("ready · next component {}", next_index + 1)
        }
        ValueRunStatus::Interrupted { step } => format!("recoverable · interrupted at {step}"),
        ValueRunStatus::CheckpointPending { step } => {
            format!("recoverable · checkpoint pending after {step}")
        }
        ValueRunStatus::Finalizing => "recoverable · finalizing".to_owned(),
    }
}

pub(super) fn total_duration(inspection: &CellInspection) -> String {
    total_duration_us(inspection).map_or_else(|| "unavailable".to_owned(), format_duration)
}

/// sum of every component's real duration, or `None` if any is missing -- never a partial total.
pub(crate) fn total_duration_us(inspection: &CellInspection) -> Option<u64> {
    inspection
        .components
        .iter()
        .try_fold(0_u64, |total, component| {
            component
                .duration_us
                .and_then(|duration| total.checked_add(duration))
        })
}

pub(super) fn format_duration(microseconds: u64) -> String {
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

pub(super) fn display_hash(hash: &str, verbose: bool) -> String {
    if verbose || !hash.is_ascii() || hash.len() <= 27 {
        return hash.to_owned();
    }
    format!("{}…{}", &hash[..15], &hash[hash.len() - 8..])
}

/// a durability reason is `"<profile id> · <human reasoning>"` -- the profile id is only useful
/// for cross-referencing a stored profile, not for answering "why did this happen," so hide it by
/// default the same way `display_hash` hides a raw component hash.
pub(crate) fn compact_reason(reason: &str, verbose: bool) -> String {
    if verbose {
        return reason.to_owned();
    }
    match reason.split_once(" · ") {
        Some((id, rest)) if id.len() >= 32 && id.bytes().all(|byte| byte.is_ascii_hexdigit()) => {
            rest.to_owned()
        }
        _ => reason.to_owned(),
    }
}

pub(super) fn marker(color: &str, symbol: &str) -> String {
    if crate::color_enabled(io::stdout().is_terminal()) {
        format!("\x1b[{color}m{symbol}\x1b[0m")
    } else {
        symbol.to_owned()
    }
}
