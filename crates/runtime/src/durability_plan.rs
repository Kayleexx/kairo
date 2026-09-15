use std::{collections::HashMap, path::Path};

use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};

use crate::{
    journal::JournalError,
    journal_event::{JournalEvent, decode_row},
};

pub const PLANNER_VERSION: &str = "v1";
const HYSTERESIS_FACTOR: u64 = 2;

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct DurabilityProfile {
    pub recompute_us: u64,
    pub checkpoint_bytes: u64,
    pub checkpoint_us: u64,
    pub samples: u32,
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

pub(crate) fn load_profile(shape: &str) -> Option<WorkflowProfile> {
    let bytes = std::fs::read(profile_path(shape)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

#[derive(Clone, Debug)]
pub struct AutoResolution {
    pub index: usize,
    pub required: bool,
    pub profile_id: String,
    pub reason: String,
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
