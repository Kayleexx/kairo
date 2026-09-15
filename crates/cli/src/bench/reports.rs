use std::path::{Path, PathBuf};

use super::report::{BenchReport, REPORT_DIRECTORY};
use super::runner::BenchError;

pub(crate) fn list() -> Result<(), BenchError> {
    let mut entries = match std::fs::read_dir(REPORT_DIRECTORY) {
        Ok(entries) => entries
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "json")
            })
            .collect::<Vec<_>>(),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(source) => {
            return Err(BenchError::WriteReport {
                path: PathBuf::from(REPORT_DIRECTORY),
                source,
            });
        }
    };
    entries.sort();
    if entries.is_empty() {
        println!("no benchmark reports · run `kairo bench run <workflow>` first");
        return Ok(());
    }
    println!("reports · {}", entries.len());
    for path in entries {
        let name = path.file_name().map_or_else(
            || path.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        );
        match load(&path) {
            Ok(report) => println!("  {name} · {}", one_line(&report)),
            Err(_) => println!("  {name} · unreadable"),
        }
    }
    Ok(())
}

pub(crate) fn show(requested: Option<&Path>) -> Result<(), BenchError> {
    let path = match requested {
        Some(requested) => resolve(requested),
        None => latest()?,
    };
    let report = load(&path)?;
    println!("{}", path.display());
    println!(
        "  workflow · {} · kairo {} · {}",
        report.config.workflow, report.kairo_version, report.platform
    );
    let scenario = report
        .config
        .failure_scenario
        .as_deref()
        .unwrap_or("timing");
    println!(
        "  warmups {} · repetitions {} · scenario {scenario}",
        report.config.warmups, report.config.repetitions
    );
    if let Some(summary) = &report.summary {
        println!(
            "\nsummary · {} successes\n  wall ms · min {} · p50 {} · p95 {} · max {} · mean {}",
            summary.successes,
            summary.wall_ms_min,
            summary.wall_ms_p50,
            summary.wall_ms_p95,
            summary.wall_ms_max,
            summary.wall_ms_mean
        );
    }
    if !report.failures.is_empty() {
        println!("\nfailures · {}", report.failures.len());
        for failure in &report.failures {
            println!("  attempt {} · {}", failure.attempt, failure.message);
        }
    }
    if !report.recovery.is_empty() {
        println!("\nrecovery cycles · {}", report.recovery.len());
        for cycle in &report.recovery {
            match (&cycle.killed_worker, cycle.recovery_wall_ms) {
                (Some(worker), Some(wall_ms)) => println!(
                    "  attempt {} · killed {worker} · recovered in {wall_ms}ms · {}",
                    cycle.attempt, cycle.outcome
                ),
                _ => println!(
                    "  attempt {} · {} (too fast to observe)",
                    cycle.attempt, cycle.outcome
                ),
            }
        }
    }
    Ok(())
}

fn one_line(report: &BenchReport) -> String {
    if !report.recovery.is_empty() {
        return format!("{} recovery cycle(s)", report.recovery.len());
    }
    match &report.summary {
        Some(summary) => format!(
            "{} successes · {} failure(s) · p50 {}ms",
            summary.successes,
            report.failures.len(),
            summary.wall_ms_p50
        ),
        None => format!("{} failure(s) · no successes", report.failures.len()),
    }
}

fn load(path: &Path) -> Result<BenchReport, BenchError> {
    let bytes = std::fs::read(path).map_err(|source| BenchError::WriteReport {
        path: path.to_path_buf(),
        source,
    })?;
    serde_json::from_slice(&bytes).map_err(|source| BenchError::Serialize { source })
}

/// a bare filename resolves under `.kairo/benchmarks/`; anything else is used as-is, matching
/// `kairo inspect`'s run-name-or-path convention.
fn resolve(requested: &Path) -> PathBuf {
    if requested.components().count() == 1 {
        Path::new(REPORT_DIRECTORY).join(requested)
    } else {
        requested.to_path_buf()
    }
}

// mtime-based, matching `state::latest`'s convention for `kairo inspect`'s default.
fn latest() -> Result<PathBuf, BenchError> {
    let entries = match std::fs::read_dir(REPORT_DIRECTORY) {
        Ok(entries) => entries,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            return Err(BenchError::NoReports);
        }
        Err(source) => {
            return Err(BenchError::WriteReport {
                path: PathBuf::from(REPORT_DIRECTORY),
                source,
            });
        }
    };
    entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .filter_map(|path| {
            let modified = path.metadata().ok()?.modified().ok()?;
            Some((modified, path))
        })
        .max_by_key(|(modified, _)| *modified)
        .map(|(_, path)| path)
        .ok_or(BenchError::NoReports)
}
