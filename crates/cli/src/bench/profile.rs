use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use kairo_control::Endpoint;
use kairo_core::{Config, Durability, WorkflowMode};
use kairo_runtime::{DurabilityProfile, Runtime, WorkflowProfile};

use super::runner::BenchError;

const PROFILE_WORKERS: usize = 2;

pub(crate) fn run(
    workflow_path: &Path,
    repetitions: u32,
    value: Option<&str>,
    allow_console: bool,
) -> Result<(PathBuf, WorkflowProfile, Vec<String>), BenchError> {
    let runtime = Runtime::new(Config {
        allow_console,
        ..Config::default()
    })?;
    let workflow = runtime.load_workflow(workflow_path)?;
    if workflow.mode() == WorkflowMode::Stream {
        return Err(BenchError::ChaosStreamUnsupported);
    }
    if workflow.mode() == WorkflowMode::Value && value.is_none() {
        return Err(BenchError::ProfileValueRequired);
    }
    let auto_steps: Vec<String> = workflow
        .steps()
        .iter()
        .enumerate()
        .filter(|(index, _)| workflow.durability_after_step(*index) == Durability::Auto)
        .map(|(_, step)| step.id.as_str().to_owned())
        .collect();
    if auto_steps.is_empty() {
        return Err(BenchError::NoAutoEdges);
    }
    let shape = if workflow.mode() == WorkflowMode::Value {
        runtime.value_workflow_shape(&workflow)?
    } else {
        runtime.workflow_shape(&workflow)?
    };

    let original = fs::read_to_string(workflow_path)
        .map_err(|source| BenchError::Spawn { attempt: 0, source })?;
    let ephemeral = write_variant(workflow_path, &original, "ephemeral")?;
    let required = write_variant(workflow_path, &original, "required")?;
    let exe = std::env::current_exe().map_err(|source| BenchError::CurrentExe { source })?;

    // one shared service for the whole profiling session, not one per internal run -- every
    // `bench-profile-N` attempt below reuses it and is forgotten from it immediately after, so no
    // internal run is ever left behind as ordinary user-visible history.
    let (endpoint, mut local) = kairo_control::ensure_endpoint(
        Path::new(".kairo"),
        None,
        PROFILE_WORKERS,
        allow_console,
        false,
    )?;
    forget_stale_profile_runs(&endpoint);

    let mut edges = HashMap::new();
    let mut outcome: Result<(), BenchError> = Ok(());
    for step in &auto_steps {
        outcome = (|| {
            let recompute_us = mean_duration(
                &endpoint,
                &exe,
                &ephemeral,
                step,
                repetitions,
                value,
                workflow.mode(),
                allow_console,
            )?;
            let (checkpoint_bytes, checkpoint_us) = mean_checkpoint(
                &endpoint,
                &exe,
                &required,
                step,
                repetitions,
                value,
                workflow.mode(),
                allow_console,
            )?;
            edges.insert(
                step.clone(),
                DurabilityProfile {
                    recompute_us,
                    checkpoint_bytes,
                    checkpoint_us,
                    samples: repetitions,
                },
            );
            Ok(())
        })();
        if outcome.is_err() {
            break;
        }
    }

    // stop on both success and failure -- an aborted profiling run must never leave an orphaned
    // ephemeral service behind any more than it leaves orphaned run records behind.
    if let Some(local) = &mut local {
        let _ = local.stop();
    }
    let _ = fs::remove_file(&ephemeral);
    let _ = fs::remove_file(&required);
    outcome?;

    let profile = WorkflowProfile {
        workflow: workflow.name().to_owned(),
        shape: shape.clone(),
        edges,
    };
    let path = write_profile(&shape, &profile)?;
    Ok((path, profile, auto_steps))
}

/// a defensive sweep for runs a previous, abnormally-terminated profiling invocation left
/// registered -- normal invocations never need this, since every attempt below forgets itself
/// right after use, but a hard kill mid-profile could otherwise leave one behind forever.
fn forget_stale_profile_runs(endpoint: &Endpoint) {
    let Ok(snapshot) = kairo_control::snapshot(endpoint) else {
        return;
    };
    for run in snapshot.runs {
        if run.id.starts_with("bench-profile-") {
            let _ = kairo_control::forget(endpoint, run.id);
        }
    }
}

fn write_variant(original_path: &Path, text: &str, target: &str) -> Result<PathBuf, BenchError> {
    let rewritten = text.replace("durability: auto", &format!("durability: {target}"));
    let path = original_path.with_extension(format!("profile-{target}.yaml"));
    fs::write(&path, rewritten).map_err(|source| BenchError::WriteReport {
        path: path.clone(),
        source,
    })?;
    Ok(path)
}

struct ProfiledComponent {
    name: String,
    duration_us: Option<u64>,
    checkpoint_bytes: Option<u64>,
    checkpoint_duration_us: Option<u64>,
}

#[allow(clippy::too_many_arguments)]
fn mean_duration(
    endpoint: &Endpoint,
    exe: &Path,
    workflow: &Path,
    step: &str,
    repetitions: u32,
    value: Option<&str>,
    mode: WorkflowMode,
    allow_console: bool,
) -> Result<u64, BenchError> {
    let mut total = 0_u64;
    let mut count = 0_u64;
    for attempt in 0..repetitions {
        let components = run_once(endpoint, exe, workflow, attempt, value, mode, allow_console)?;
        if let Some(component) = components.iter().find(|component| component.name == step)
            && let Some(duration_us) = component.duration_us
        {
            total += duration_us;
            count += 1;
        }
    }
    Ok(total.checked_div(count).unwrap_or(0))
}

#[allow(clippy::too_many_arguments)]
fn mean_checkpoint(
    endpoint: &Endpoint,
    exe: &Path,
    workflow: &Path,
    step: &str,
    repetitions: u32,
    value: Option<&str>,
    mode: WorkflowMode,
    allow_console: bool,
) -> Result<(u64, u64), BenchError> {
    let (mut bytes_total, mut duration_total, mut count) = (0_u64, 0_u64, 0_u64);
    for attempt in 0..repetitions {
        let components = run_once(endpoint, exe, workflow, attempt, value, mode, allow_console)?;
        if let Some(component) = components.iter().find(|component| component.name == step)
            && let (Some(bytes), Some(duration_us)) =
                (component.checkpoint_bytes, component.checkpoint_duration_us)
        {
            bytes_total += bytes;
            duration_total += duration_us;
            count += 1;
        }
    }
    if count == 0 {
        return Ok((0, 0));
    }
    Ok((bytes_total / count, duration_total / count))
}

fn run_once(
    endpoint: &Endpoint,
    exe: &Path,
    workflow: &Path,
    attempt: u32,
    value: Option<&str>,
    mode: WorkflowMode,
    allow_console: bool,
) -> Result<Vec<ProfiledComponent>, BenchError> {
    let run_name = format!("bench-profile-{attempt}");
    let path = PathBuf::from(".kairo").join(format!("{run_name}.db"));
    let mut command = Command::new(exe);
    if allow_console {
        command.arg("--allow-console");
    }
    command.arg("run").arg(workflow).args(["--run", &run_name]);
    if let Some(value) = value {
        command.args(["--value", value]);
    }
    let output = command
        .output()
        .map_err(|source| BenchError::Spawn { attempt, source });
    let result = match output {
        Ok(output) if output.status.success() => profiled_components(&path, mode, attempt),
        Ok(output) => {
            let message = String::from_utf8_lossy(&output.stderr).trim().to_owned();
            Err(BenchError::ProfileRunFailed { attempt, message })
        }
        Err(error) => Err(error),
    };
    // forgotten regardless of outcome -- a failed internal attempt still registered a (now
    // terminal) run on the control service and must not linger there either.
    let _ = kairo_control::forget(endpoint, run_name);
    cleanup(&path);
    result
}

fn profiled_components(
    path: &Path,
    mode: WorkflowMode,
    attempt: u32,
) -> Result<Vec<ProfiledComponent>, BenchError> {
    let not_completed = || BenchError::ProfileRunFailed {
        attempt,
        message: "run did not reach a completed state".to_owned(),
    };
    if mode == WorkflowMode::Value {
        let inspection = crate::inspection::inspect_aggregated_value(path)?.ok_or_else(|| {
            BenchError::ProfileRunFailed {
                attempt,
                message: "run produced a scalar journal for a value-mode workflow".to_owned(),
            }
        })?;
        if !matches!(
            inspection.status,
            kairo_runtime::ValueRunStatus::Completed { .. }
        ) {
            return Err(not_completed());
        }
        return Ok(inspection
            .components
            .iter()
            .map(|component| ProfiledComponent {
                name: component.name.clone(),
                duration_us: component.duration_us,
                checkpoint_bytes: component.checkpoint_bytes,
                checkpoint_duration_us: component.checkpoint_duration_us,
            })
            .collect());
    }
    let inspection = crate::inspection::inspect_aggregated(path)?;
    if !matches!(
        inspection.status,
        kairo_runtime::CellStatus::Completed { .. }
    ) {
        return Err(not_completed());
    }
    Ok(inspection
        .components
        .iter()
        .map(|component| ProfiledComponent {
            name: component.name.clone(),
            duration_us: component.duration_us,
            checkpoint_bytes: component.checkpoint_bytes,
            checkpoint_duration_us: component.checkpoint_duration_us,
        })
        .collect())
}

fn cleanup(path: &Path) {
    let mut journals = vec![path.to_path_buf()];
    journals.extend(crate::inspection::sibling_groups(path));
    for journal in journals {
        for extension in ["db", "db-shm", "db-wal", "lock"] {
            let _ = fs::remove_file(journal.with_extension(extension));
        }
    }
}

fn write_profile(shape: &str, profile: &WorkflowProfile) -> Result<PathBuf, BenchError> {
    let directory = Path::new(".kairo/profiles");
    fs::create_dir_all(directory).map_err(|source| BenchError::WriteReport {
        path: directory.to_path_buf(),
        source,
    })?;
    let path = directory.join(format!("{shape}.json"));
    let bytes =
        serde_json::to_vec_pretty(profile).map_err(|source| BenchError::Serialize { source })?;
    fs::write(&path, bytes).map_err(|source| BenchError::WriteReport {
        path: path.clone(),
        source,
    })?;
    Ok(path)
}
