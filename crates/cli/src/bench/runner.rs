use std::{path::PathBuf, process::Command, time::Instant};

use kairo_core::{Workflow, WorkflowMode};
use kairo_runtime::{CellStatus, StreamRunStatus};
use thiserror::Error;

use super::{
    chaos,
    config::BenchConfig,
    report::{self, BenchReport, EdgeSample, FailureRecord, Sample},
};

#[derive(Debug, Error)]
pub(crate) enum BenchError {
    #[error("failed to load workflow `{path}`")]
    Workflow {
        path: PathBuf,
        #[source]
        source: kairo_core::WorkflowError,
    },
    #[error(
        "benchmarking a stream workflow with a failure scenario isn't supported yet -- stream workflows don't go through the control plane"
    )]
    ChaosStreamUnsupported,
    #[error("benchmarking a `mode: value` workflow isn't supported yet")]
    ValueUnsupported,
    #[error("failed to determine the current executable path")]
    CurrentExe {
        #[source]
        source: std::io::Error,
    },
    #[error("failed to run attempt {attempt}")]
    Spawn {
        attempt: u32,
        #[source]
        source: std::io::Error,
    },
    #[error(transparent)]
    Inspect(#[from] kairo_runtime::JournalError),
    #[error(transparent)]
    InspectStream(#[from] kairo_runtime::StreamRunError),
    #[error("no stream run was recorded at `{path}`")]
    NoStreamRun { path: PathBuf },
    #[error("benchmark report already exists at `{path}`")]
    ReportExists { path: PathBuf },
    #[error("failed to write benchmark report `{path}`")]
    WriteReport {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to serialize benchmark report")]
    Serialize {
        #[source]
        source: serde_json::Error,
    },
    #[error("timed out waiting for run `{id}` during the worker-kill scenario")]
    ChaosTimeout { id: String },
    #[error(transparent)]
    Control(#[from] kairo_control::ControlError),
    #[error(transparent)]
    StatePath(#[from] crate::state::StateError),
    // boxed: CliError embeds BenchError, so this side must not embed CliError unboxed.
    #[error(transparent)]
    Cli(Box<crate::CliError>),
    #[error(transparent)]
    Runtime(#[from] kairo_runtime::RuntimeError),
    #[error("workflow has no `durability: auto` edges to profile")]
    NoAutoEdges,
    #[error(transparent)]
    Discovery(#[from] crate::discovery::DiscoveryError),
    #[error("no benchmark reports · run `kairo bench run <workflow>` first")]
    NoReports,
    #[error("profile attempt {attempt} failed: {message}")]
    ProfileRunFailed { attempt: u32, message: String },
    #[error("profiling a `mode: value` workflow needs a literal value; pass `--value <text>`")]
    ProfileValueRequired,
}

impl From<crate::CliError> for BenchError {
    fn from(error: crate::CliError) -> Self {
        Self::Cli(Box::new(error))
    }
}

pub(crate) fn run(config: BenchConfig) -> Result<PathBuf, BenchError> {
    let workflow = Workflow::load(
        &config.workflow,
        kairo_core::Config::default().max_workflow_bytes,
        kairo_core::Config::default().max_workflow_steps,
    )
    .map_err(|source| BenchError::Workflow {
        path: config.workflow.clone(),
        source,
    })?;

    if config.failure_scenario.is_some() {
        if workflow.mode() != WorkflowMode::Scalar {
            return Err(BenchError::ChaosStreamUnsupported);
        }
        let mut recovery = Vec::new();
        for attempt in 0..config.repetitions {
            recovery.push(chaos::run_worker_kill_cycle(
                &config.workflow,
                &workflow,
                attempt,
            )?);
        }
        let report = BenchReport::new(&config, Vec::new(), Vec::new(), recovery);
        return report::write(&config, &report);
    }

    let exe = std::env::current_exe().map_err(|source| BenchError::CurrentExe { source })?;
    let mut raw_samples = Vec::new();
    let mut failures = Vec::new();
    let total = config.warmups.saturating_add(config.repetitions);
    for attempt in 0..total {
        let warmup = attempt < config.warmups;
        let outcome = run_attempt(&exe, &config, workflow.mode(), workflow.name(), attempt)?;
        match outcome {
            Attempt::Failed(message) => {
                if !warmup {
                    failures.push(FailureRecord { attempt, message });
                }
            }
            Attempt::Succeeded(sample) if !warmup => raw_samples.push(sample),
            Attempt::Succeeded(_) => {}
        }
    }

    let report = BenchReport::new(&config, raw_samples, failures, Vec::new());
    report::write(&config, &report)
}

enum Attempt {
    Succeeded(Sample),
    Failed(String),
}

fn run_attempt(
    exe: &std::path::Path,
    config: &BenchConfig,
    mode: WorkflowMode,
    workflow_name: &str,
    attempt: u32,
) -> Result<Attempt, BenchError> {
    let run_name = format!("bench-{}-{attempt}", sanitize(workflow_name));
    let mut command = Command::new(exe);
    command.arg("run").arg(&config.workflow);
    if let Some(input) = &config.input {
        command.arg(input);
    }
    command.args(["--run", &run_name]);

    let started = Instant::now();
    let output = command
        .output()
        .map_err(|source| BenchError::Spawn { attempt, source })?;
    let wall_ms = started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;

    // `run_name` is built entirely from `sanitize()` output plus a numeric attempt, so it's
    // always a valid run id -- this mirrors `state::resolve_run`'s own cell-name path shape.
    let path = std::path::PathBuf::from(".kairo").join(format!("{run_name}.db"));

    if !output.status.success() {
        let message = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        cleanup(&path);
        return Ok(Attempt::Failed(message));
    }

    let outcome = match mode {
        WorkflowMode::Scalar => scalar_sample(&path, attempt, wall_ms)?,
        WorkflowMode::Stream => stream_sample(&path, attempt, wall_ms)?,
        WorkflowMode::Value => {
            cleanup(&path);
            return Err(BenchError::ValueUnsupported);
        }
    };
    cleanup(&path);
    Ok(outcome)
}

fn scalar_sample(
    path: &std::path::Path,
    attempt: u32,
    wall_ms: u64,
) -> Result<Attempt, BenchError> {
    let inspection = crate::inspection::inspect_aggregated(path)?;
    let workflow_duration_us = crate::inspection::total_duration_us(&inspection);
    Ok(match inspection.status {
        CellStatus::Completed { output } => Attempt::Succeeded(Sample {
            attempt,
            wall_ms,
            workflow_duration_us,
            output: Some(output),
            stream_bytes: None,
            materialized_bytes: None,
            edges: Vec::new(),
        }),
        _ => Attempt::Failed("run did not reach a completed state".to_owned()),
    })
}

fn stream_sample(
    path: &std::path::Path,
    attempt: u32,
    wall_ms: u64,
) -> Result<Attempt, BenchError> {
    let inspection =
        kairo_runtime::inspect_stream_run(path)?.ok_or_else(|| BenchError::NoStreamRun {
            path: path.to_path_buf(),
        })?;
    Ok(match inspection.status {
        StreamRunStatus::Completed => {
            let edges = inspection
                .metrics
                .as_ref()
                .map(|metrics| {
                    metrics
                        .edges
                        .iter()
                        .map(|edge| EdgeSample {
                            name: edge.name.clone(),
                            bytes: edge.bytes,
                            peak_buffered_bytes: edge.peak_buffered_bytes,
                            materialized: edge.materialized,
                        })
                        .collect()
                })
                .unwrap_or_default();
            Attempt::Succeeded(Sample {
                attempt,
                wall_ms,
                workflow_duration_us: inspection.duration_us,
                output: None,
                stream_bytes: inspection
                    .metrics
                    .as_ref()
                    .map(|metrics| metrics.source_bytes),
                materialized_bytes: inspection
                    .metrics
                    .as_ref()
                    .map(|metrics| metrics.materialized_bytes),
                edges,
            })
        }
        StreamRunStatus::Failed(message) => Attempt::Failed(message),
        StreamRunStatus::Running => {
            Attempt::Failed("run did not reach a completed state".to_owned())
        }
    })
}

/// benchmark runs are throwaway -- clean their journal files up immediately rather than
/// accumulating alongside real runs (mirrors `kairo prune`'s file set).
fn cleanup(path: &std::path::Path) {
    let mut journals = vec![path.to_path_buf()];
    journals.extend(crate::inspection::sibling_groups(path));
    for journal in journals {
        for extension in ["db", "db-shm", "db-wal", "lock"] {
            let _ = std::fs::remove_file(journal.with_extension(extension));
        }
    }
}

fn sanitize(name: &str) -> String {
    name.chars()
        .map(|character| {
            if character.is_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect()
}
