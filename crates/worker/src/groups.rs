use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use kairo_control::{Endpoint, RunEvent, RunOutcome, RunPlan, RunRequest, Snapshot, WorkerResult};
use kairo_core::{Durability, Workflow, WorkflowMode, plan_groups};
use kairo_runtime::{AutoResolution, GroupOutcome, Runtime};
use kairo_storage::ArtifactStore;

/// below this, a boundary's committed artifact is cheap enough to hand to another worker;
/// at or above it, kairo.md's "prefer co-location when several GB move" rule keeps it local.
const HEAVY_EDGE_BYTES_THRESHOLD: u64 = 1024 * 1024;

#[allow(clippy::too_many_arguments)]
pub(crate) fn execute(
    executor: &tokio::runtime::Runtime,
    runtime: &Runtime,
    workflow: &Workflow,
    run: &RunRequest,
    artifacts: Option<&ArtifactStore>,
    endpoint: &Endpoint,
    self_worker: &str,
    epoch: u64,
) -> Result<WorkerResult, String> {
    if workflow.mode() == WorkflowMode::Stream {
        return super::stream_groups::execute(
            executor,
            runtime,
            workflow,
            run,
            artifacts,
            endpoint,
            self_worker,
            epoch,
        );
    }
    let (resolved, auto_plan, shape) = match &run.plan {
        Some(plan) => (plan.resolved_durability.clone(), Vec::new(), None),
        None => {
            let (auto_resolved, auto_plan) = runtime
                .resolve_run_plan(workflow, &run.state)
                .map_err(|error| error.to_string())?;
            let shape = runtime
                .workflow_shape(workflow)
                .map_err(|error| error.to_string())?;
            // `resolve_run_plan` only decides `durability: auto` edges -- a legal group boundary
            // is any durably-required edge, so combine those decisions with every declared
            // `required` edge into the one full per-step map `plan_groups` and every group's own
            // `Cell::open` actually need.
            let full_resolved = (0..workflow.steps().len())
                .map(|index| {
                    let required = match workflow.durability_after_step(index) {
                        Durability::Required => true,
                        Durability::Ephemeral => false,
                        Durability::Auto => auto_resolved.get(&index).copied().unwrap_or(false),
                    };
                    (index, required)
                })
                .collect::<BTreeMap<_, _>>();
            (full_resolved, auto_plan, Some(shape))
        }
    };

    let groups = plan_groups(workflow, &resolved);
    let (start_index, start_input) = match &run.resume {
        Some(resume) => {
            let store = artifacts.ok_or("resuming an execution group needs artifact storage")?;
            let artifact = executor
                .block_on(store.get(&resume.artifact_hash))
                .map_err(|error| error.to_string())?;
            (resume.from_index, artifact.value)
        }
        None => (
            0,
            workflow
                .scalar_input()
                .ok_or("scalar workflow input is required")?,
        ),
    };
    let group = groups
        .iter()
        .find(|group| group.start_index == start_index)
        .ok_or("execution group boundary mismatch")?;
    let is_last_group = group.end_index + 1 == workflow.steps().len();
    let stop_at = (!is_last_group).then_some(group.end_index);
    let group_auto_plan = auto_plan_for_group(
        workflow,
        &resolved,
        &auto_plan,
        group.start_index,
        group.end_index,
    );
    let state_path = group_state_path(&run.state, start_index);

    let outcome = executor
        .block_on(runtime.run_cell_group(
            workflow,
            &state_path,
            artifacts,
            &resolved,
            &group_auto_plan,
            start_index,
            start_input,
            stop_at,
        ))
        .map_err(|error| error.to_string())?;

    // never fails this run over a profile write-back problem -- logged and ignored, exactly
    // like a missed metrics sample would be.
    if let Err(error) = runtime.record_profile_observations(workflow, &state_path) {
        tracing::warn!(%error, "failed to record durability profile observations");
    }

    match outcome {
        GroupOutcome::Completed { output } => Ok(WorkerResult::Completed(
            kairo_control::RunOutput::Scalar(output),
        )),
        GroupOutcome::Yielded {
            next_index,
            artifact_hash,
            artifact_backend,
        } => {
            let checkpoint_bytes = runtime
                .edge_checkpoint_bytes(workflow, group.end_index)
                .map_err(|error| error.to_string())?;
            let storage_is_shared = run.storage.as_ref().is_some_and(|config| !config.local);
            let snapshot = kairo_control::snapshot(endpoint).map_err(|error| error.to_string())?;
            let facts = worker_facts(&snapshot, shape.as_deref().or(run.shape.as_deref()));
            let placement = decide_placement(
                self_worker,
                storage_is_shared,
                checkpoint_bytes,
                HEAVY_EDGE_BYTES_THRESHOLD,
                &facts,
            );
            let (target_worker, target_had_cache) = match placement {
                Placement::Stay => (None, false),
                Placement::Move {
                    target,
                    target_had_cache,
                } => (Some(target), target_had_cache),
            };
            Ok(WorkerResult::Yielded {
                next_index,
                artifact_hash,
                artifact_backend,
                plan: run.plan.is_none().then(|| RunPlan {
                    resolved_durability: resolved.clone(),
                }),
                shape,
                target_worker,
                target_had_cache,
            })
        }
    }
}

/// the auto-durability resolutions relevant to one group's own step range, with real reasons
/// when freshly resolved, or a note that they were already decided by an earlier group when
/// reusing a persisted plan -- never re-derived either way.
fn auto_plan_for_group(
    workflow: &Workflow,
    resolved: &BTreeMap<usize, bool>,
    auto_plan: &[AutoResolution],
    start_index: usize,
    end_index: usize,
) -> Vec<AutoResolution> {
    let in_range: Vec<AutoResolution> = auto_plan
        .iter()
        .filter(|resolution| resolution.index >= start_index && resolution.index <= end_index)
        .cloned()
        .collect();
    if !in_range.is_empty() {
        return in_range;
    }
    (start_index..=end_index)
        .filter(|&index| workflow.durability_after_step(index) == Durability::Auto)
        .filter_map(|index| {
            resolved.get(&index).map(|&required| AutoResolution {
                index,
                required,
                profile_id: "propagated".to_owned(),
                reason: "resolved by an earlier group in this run".to_owned(),
                profile: None,
            })
        })
        .collect()
}

pub struct WorkerFacts<'a> {
    pub id: &'a str,
    pub healthy: bool,
    pub idle: bool,
    pub recent_for_shape: bool,
}

#[derive(Debug, Eq, PartialEq)]
pub enum Placement {
    Stay,
    Move {
        target: String,
        target_had_cache: bool,
    },
}

/// kairo.md Section 18's three-gate rule: satisfy resource requirements, then prefer
/// co-location for heavy edges, then pick deterministically among what's left.
pub fn decide_placement(
    self_worker: &str,
    storage_is_shared: bool,
    checkpoint_bytes: Option<u64>,
    threshold_bytes: u64,
    candidates: &[WorkerFacts<'_>],
) -> Placement {
    if !storage_is_shared {
        return Placement::Stay;
    }
    let mut eligible: Vec<&WorkerFacts<'_>> = candidates
        .iter()
        .filter(|candidate| candidate.id != self_worker && candidate.healthy && candidate.idle)
        .collect();
    if eligible.is_empty() {
        return Placement::Stay;
    }
    // unknown or heavy: stay co-located, never move data just because a worker happens to be idle.
    let Some(bytes) = checkpoint_bytes else {
        return Placement::Stay;
    };
    if bytes >= threshold_bytes {
        return Placement::Stay;
    }
    eligible.sort_by(|left, right| {
        right
            .recent_for_shape
            .cmp(&left.recent_for_shape)
            .then_with(|| left.id.cmp(right.id))
    });
    let best = eligible[0];
    Placement::Move {
        target: best.id.to_owned(),
        target_had_cache: best.recent_for_shape,
    }
}

fn worker_facts<'a>(snapshot: &'a Snapshot, shape: Option<&str>) -> Vec<WorkerFacts<'a>> {
    let recent_worker = shape.and_then(|shape| most_recently_completed_worker(snapshot, shape));
    snapshot
        .workers
        .iter()
        .map(|worker| WorkerFacts {
            id: worker.id.as_str(),
            healthy: worker.healthy,
            idle: !worker.busy,
            recent_for_shape: recent_worker == Some(worker.id.as_str()),
        })
        .collect()
}

fn most_recently_completed_worker<'a>(snapshot: &'a Snapshot, shape: &str) -> Option<&'a str> {
    snapshot
        .runs
        .iter()
        .filter(|run| run.shape.as_deref() == Some(shape))
        .flat_map(|run| run.history.iter())
        .filter_map(|event| match event {
            RunEvent::Outcome {
                worker,
                at_ms,
                outcome: RunOutcome::Completed,
                ..
            } => Some((worker.as_str(), *at_ms)),
            _ => None,
        })
        .max_by_key(|&(_, at_ms)| at_ms)
        .map(|(worker, _)| worker)
}

pub fn group_state_path(base: &Path, start_index: usize) -> PathBuf {
    if start_index == 0 {
        return base.to_path_buf();
    }
    let mut name = base
        .file_stem()
        .map(std::ffi::OsStr::to_os_string)
        .unwrap_or_default();
    name.push(format!(".group-{start_index}"));
    if let Some(extension) = base.extension() {
        name.push(".");
        name.push(extension);
    }
    base.with_file_name(name)
}
