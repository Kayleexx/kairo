use std::{
    collections::HashMap,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};

use crate::{
    journal::JournalError,
    journal_event::{JournalEvent, decode_row},
};

pub const PLANNER_VERSION: &str = "v1";
const HYSTERESIS_FACTOR: u64 = 2;
/// bounded window of the most recent real recompute measurements kept per edge -- enough for a
/// real p50/p90, never an unbounded measurement history.
const RECOMPUTE_WINDOW: usize = 32;
/// a profile with fewer real samples than this is still used (never an error), but `ensure_profiled`
/// tops it up with one more real measurement rather than trusting a single sample forever.
pub const MIN_TRUSTED_SAMPLES: u32 = 3;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DurabilityProfile {
    pub recompute_us: u64,
    #[serde(default)]
    pub recompute_p50_us: u64,
    #[serde(default)]
    pub recompute_p90_us: u64,
    pub checkpoint_bytes: u64,
    pub checkpoint_us: u64,
    /// real count of recompute measurements folded into this profile, never a requested
    /// repetition count.
    pub samples: u32,
    #[serde(default)]
    pub checkpoint_samples: u32,
    #[serde(default = "now_ms")]
    pub recorded_at_ms: u64,
    #[serde(default)]
    pub worker_id: Option<String>,
    #[serde(default)]
    pub failures: u32,
    #[serde(default)]
    pub recompute_samples_us: Vec<u64>,
}

impl DurabilityProfile {
    pub fn empty(now_ms: u64) -> Self {
        Self {
            recompute_us: 0,
            recompute_p50_us: 0,
            recompute_p90_us: 0,
            checkpoint_bytes: 0,
            checkpoint_us: 0,
            samples: 0,
            checkpoint_samples: 0,
            recorded_at_ms: now_ms,
            worker_id: None,
            failures: 0,
            recompute_samples_us: Vec::new(),
        }
    }

    /// folds one more real recompute measurement in, keeping only the most recent
    /// `RECOMPUTE_WINDOW` samples -- `recompute_us`/`_p50_us`/`_p90_us` are always derived fresh
    /// from that bounded window, never accumulated as a running approximation.
    #[must_use]
    pub fn with_recompute_sample(
        &self,
        recompute_us: u64,
        worker_id: Option<&str>,
        now_ms: u64,
    ) -> Self {
        let mut window = self.recompute_samples_us.clone();
        window.push(recompute_us);
        if window.len() > RECOMPUTE_WINDOW {
            window.remove(0);
        }
        let (p50, p90) = percentiles(&window);
        #[allow(clippy::cast_possible_truncation)]
        let mean = window.iter().sum::<u64>() / window.len() as u64;
        Self {
            recompute_us: mean,
            recompute_p50_us: p50,
            recompute_p90_us: p90,
            samples: self.samples.saturating_add(1),
            recorded_at_ms: now_ms,
            worker_id: worker_id
                .map(str::to_owned)
                .or_else(|| self.worker_id.clone()),
            recompute_samples_us: window,
            ..self.clone()
        }
    }

    /// folds one more real checkpoint measurement in as an incremental mean -- checkpoint size and
    /// duration for a given edge are far less variable run to run than recompute time, so a running
    /// mean is enough; no percentile window is kept for it.
    #[must_use]
    pub fn with_checkpoint_sample(&self, bytes: u64, duration_us: u64, now_ms: u64) -> Self {
        Self {
            checkpoint_bytes: incremental_mean(
                self.checkpoint_bytes,
                self.checkpoint_samples,
                bytes,
            ),
            checkpoint_us: incremental_mean(
                self.checkpoint_us,
                self.checkpoint_samples,
                duration_us,
            ),
            checkpoint_samples: self.checkpoint_samples.saturating_add(1),
            recorded_at_ms: now_ms,
            ..self.clone()
        }
    }

    #[must_use]
    pub fn with_failure(&self, now_ms: u64) -> Self {
        Self {
            failures: self.failures.saturating_add(1),
            recorded_at_ms: now_ms,
            ..self.clone()
        }
    }
}

fn incremental_mean(old_mean: u64, old_count: u32, new_value: u64) -> u64 {
    if old_count == 0 {
        return new_value;
    }
    let count = i128::from(old_count) + 1;
    let delta = i128::from(new_value) - i128::from(old_mean);
    let updated = i128::from(old_mean) + delta / count;
    updated.clamp(0, i128::from(u64::MAX)) as u64
}

fn percentiles(samples: &[u64]) -> (u64, u64) {
    if samples.is_empty() {
        return (0, 0);
    }
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    (percentile(&sorted, 50), percentile(&sorted, 90))
}

fn percentile(sorted: &[u64], pct: usize) -> u64 {
    let rank = sorted.len().saturating_sub(1) * pct / 100;
    sorted[rank]
}

pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct WorkflowProfile {
    pub workflow: String,
    pub shape: String,
    pub edges: HashMap<String, DurabilityProfile>,
}

/// exposed so callers that already hold a measured `DurabilityProfile` (e.g. `kairo workflow
/// profile`'s own summary) can show the exact same decision `kairo run`/`kairo inspect` act on,
/// without duplicating the hysteresis rule.
pub fn decide(profile: &DurabilityProfile) -> (bool, String) {
    let threshold = profile.checkpoint_us.saturating_mul(HYSTERESIS_FACTOR);
    let required = profile.recompute_us > threshold;
    let reason = format!(
        "recompute {}us {} {}x checkpoint {}us ({} bytes, {} sample(s))",
        profile.recompute_us,
        if required { ">" } else { "<=" },
        HYSTERESIS_FACTOR,
        profile.checkpoint_us,
        profile.checkpoint_bytes,
        profile.samples,
    );
    (required, reason)
}

pub(crate) fn profile_path(shape: &str) -> std::path::PathBuf {
    Path::new(".kairo/profiles").join(format!("{shape}.json"))
}

pub fn load_profile(shape: &str) -> Option<WorkflowProfile> {
    let bytes = std::fs::read(profile_path(shape)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

pub(crate) fn save_profile(profile: &WorkflowProfile) -> std::io::Result<()> {
    let directory = Path::new(".kairo/profiles");
    std::fs::create_dir_all(directory)?;
    let bytes = serde_json::to_vec_pretty(profile)?;
    std::fs::write(profile_path(&profile.shape), bytes)
}

/// generic over `ComponentInspection`/`ValueComponentInspection` via `extract` -- both modes fold
/// their own real per-run measurements back into the same `.kairo/profiles/<shape>.json` shape,
/// for every step whose edge is `durability: auto`, using only what this run actually observed.
pub(crate) fn record_observations<T>(
    workflow: &kairo_core::Workflow,
    shape: &str,
    components: &[T],
    extract: impl Fn(&T) -> (usize, Option<u64>, Option<bool>, Option<u64>, Option<u64>),
) -> std::io::Result<()> {
    let mut profile = load_profile(shape).unwrap_or_else(|| WorkflowProfile {
        workflow: workflow.name().to_owned(),
        shape: shape.to_owned(),
        edges: HashMap::new(),
    });
    let now = now_ms();
    let mut changed = false;
    for component in components {
        let (index, duration_us, durable_after, checkpoint_bytes, checkpoint_duration_us) =
            extract(component);
        if workflow.durability_after_step(index) != kairo_core::Durability::Auto {
            continue;
        }
        let Some(step) = workflow.steps().get(index) else {
            continue;
        };
        match durable_after {
            Some(true) => {
                if let (Some(bytes), Some(duration_us)) = (checkpoint_bytes, checkpoint_duration_us)
                {
                    let entry = profile
                        .edges
                        .entry(step.id.to_string())
                        .or_insert_with(|| DurabilityProfile::empty(now));
                    *entry = entry.with_checkpoint_sample(bytes, duration_us, now);
                    changed = true;
                }
            }
            Some(false) => {
                if let Some(duration_us) = duration_us {
                    let entry = profile
                        .edges
                        .entry(step.id.to_string())
                        .or_insert_with(|| DurabilityProfile::empty(now));
                    *entry = entry.with_recompute_sample(duration_us, None, now);
                    changed = true;
                }
            }
            None => {}
        }
    }
    if changed {
        save_profile(&profile)
    } else {
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct AutoResolution {
    pub index: usize,
    pub required: bool,
    pub profile_id: String,
    pub reason: String,
    /// the profile numbers actually compared for this resolution -- `None` when a resolution is
    /// merely propagated from an earlier group's already-persisted decision (`groups.rs`'s
    /// `auto_plan_for_group`), which has the boolean result but not the original measurements.
    /// Never a fabricated/zeroed profile.
    pub profile: Option<DurabilityProfile>,
}

// read-only, no lock -- lets a resumed run reuse its original plan instead of recalculating from
// a since-changed profile, without deadlocking against `Journal::open`'s own exclusive lock.
pub(crate) fn peek_plan(state_path: &Path) -> Result<Option<HashMap<usize, bool>>, JournalError> {
    if !state_path.exists() {
        return Ok(None);
    }
    let connection = Connection::open_with_flags(state_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|source| JournalError::Open { source })?;
    let version = connection
        .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
        .map_err(|source| JournalError::Read { source })?;
    let query = crate::inspection::event_query(version)?;
    let mut statement = connection
        .prepare(&query)
        .map_err(|source| JournalError::Read { source })?;
    let mut rows = statement
        .query([])
        .map_err(|source| JournalError::Read { source })?;
    let mut plan = HashMap::new();
    while let Some(row) = rows
        .next()
        .map_err(|source| JournalError::Read { source })?
    {
        let (_, event) = decode_row(row)?;
        if let JournalEvent::DurabilityPlanned {
            index, required, ..
        } = event
        {
            plan.insert(index, required);
        }
    }
    Ok(Some(plan))
}
