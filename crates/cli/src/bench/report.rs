use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

use super::{config::BenchConfig, runner::BenchError};

pub(crate) const REPORT_DIRECTORY: &str = ".kairo/benchmarks";

#[derive(Serialize, Deserialize)]
pub(crate) struct BenchReport {
    pub(crate) kairo_version: String,
    pub(crate) platform: String,
    pub(crate) generated_at_ms: u64,
    pub(crate) config: BenchConfigSummary,
    pub(crate) raw_samples: Vec<Sample>,
    pub(crate) summary: Option<Summary>,
    pub(crate) failures: Vec<FailureRecord>,
    pub(crate) recovery: Vec<RecoveryRecord>,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct BenchConfigSummary {
    pub(crate) workflow: String,
    pub(crate) input: Option<String>,
    pub(crate) warmups: u32,
    pub(crate) repetitions: u32,
    pub(crate) failure_scenario: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct Sample {
    pub(crate) attempt: u32,
    pub(crate) wall_ms: u64,
    /// real sum of every component's recorded duration; `None` when unavailable, never faked.
    pub(crate) workflow_duration_us: Option<u64>,
    pub(crate) output: Option<u32>,
    pub(crate) stream_bytes: Option<u64>,
    pub(crate) materialized_bytes: Option<u64>,
    pub(crate) edges: Vec<EdgeSample>,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct EdgeSample {
    pub(crate) name: String,
    pub(crate) bytes: Option<u64>,
    pub(crate) peak_buffered_bytes: Option<u64>,
    pub(crate) materialized: Option<bool>,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct Summary {
    pub(crate) successes: u32,
    pub(crate) wall_ms_min: u64,
    pub(crate) wall_ms_max: u64,
    pub(crate) wall_ms_mean: u64,
    pub(crate) wall_ms_p50: u64,
    pub(crate) wall_ms_p95: u64,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct FailureRecord {
    pub(crate) attempt: u32,
    pub(crate) message: String,
}

/// one real submit-kill-recover cycle. `killed_worker`/`recovery_wall_ms` stay `None` when the
/// run finished before a worker could be observed and killed -- not a failure, just too fast to
/// sample, and never reported as a fabricated recovery time.
#[derive(Serialize, Deserialize)]
pub(crate) struct RecoveryRecord {
    pub(crate) attempt: u32,
    pub(crate) killed_worker: Option<String>,
    pub(crate) recovery_wall_ms: Option<u64>,
    pub(crate) outcome: String,
    pub(crate) reassignment_reason: Option<String>,
}

impl BenchReport {
    pub(crate) fn new(
        config: &BenchConfig,
        raw_samples: Vec<Sample>,
        failures: Vec<FailureRecord>,
        recovery: Vec<RecoveryRecord>,
    ) -> Self {
        Self {
            kairo_version: env!("CARGO_PKG_VERSION").to_owned(),
            platform: format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH),
            generated_at_ms: now_ms(),
            config: BenchConfigSummary {
                workflow: config.workflow.display().to_string(),
                input: config
                    .input
                    .as_ref()
                    .map(|input| input.display().to_string()),
                warmups: config.warmups,
                repetitions: config.repetitions,
                failure_scenario: config.failure_scenario.map(|scenario| {
                    match scenario {
                        super::config::FailureScenario::WorkerKill => "worker-kill",
                    }
                    .to_owned()
                }),
            },
            summary: summarize(&raw_samples),
            raw_samples,
            failures,
            recovery,
        }
    }
}

fn summarize(samples: &[Sample]) -> Option<Summary> {
    if samples.is_empty() {
        return None;
    }
    let mut wall_ms: Vec<u64> = samples.iter().map(|sample| sample.wall_ms).collect();
    wall_ms.sort_unstable();
    let count = wall_ms.len() as u64;
    Some(Summary {
        successes: samples.len() as u32,
        wall_ms_min: wall_ms[0],
        wall_ms_max: wall_ms[wall_ms.len() - 1],
        wall_ms_mean: wall_ms.iter().sum::<u64>() / count,
        wall_ms_p50: percentile(&wall_ms, 50),
        wall_ms_p95: percentile(&wall_ms, 95),
    })
}

fn percentile(sorted: &[u64], target: usize) -> u64 {
    let index = (sorted.len() - 1) * target / 100;
    sorted[index]
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            elapsed.as_millis().min(u128::from(u64::MAX)) as u64
        })
}

/// writes the report as a new file; refuses to silently overwrite an existing one.
pub(crate) fn write(config: &BenchConfig, report: &BenchReport) -> Result<PathBuf, BenchError> {
    let path = match &config.output {
        Some(path) => path.clone(),
        None => default_path(config),
    };
    ensure_available(&path)?;
    let bytes =
        serde_json::to_vec_pretty(report).map_err(|source| BenchError::Serialize { source })?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|source| {
            if source.kind() == std::io::ErrorKind::AlreadyExists {
                BenchError::ReportExists { path: path.clone() }
            } else {
                BenchError::WriteReport {
                    path: path.clone(),
                    source,
                }
            }
        })?;
    file.write_all(&bytes)
        .map_err(|source| BenchError::WriteReport {
            path: path.clone(),
            source,
        })?;
    Ok(path)
}

pub(crate) fn ensure_available(path: &Path) -> Result<(), BenchError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| BenchError::WriteReport {
            path: path.to_path_buf(),
            source,
        })?;
    }
    if path.exists() {
        return Err(BenchError::ReportExists {
            path: path.to_path_buf(),
        });
    }
    Ok(())
}

fn default_path(config: &BenchConfig) -> PathBuf {
    let name = config.workflow.file_stem().map_or_else(
        || "workflow".to_owned(),
        |name| name.to_string_lossy().into_owned(),
    );
    Path::new(REPORT_DIRECTORY).join(format!("{name}-{}.json", now_ms()))
}
