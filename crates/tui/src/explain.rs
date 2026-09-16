use kairo_control::{AssignmentReason, RunEvent};
use kairo_runtime::{
    CellInspection, ComponentInspection, ValueComponentInspection, ValueRunInspection,
};
use serde::Serialize;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", content = "value")]
pub enum Metric {
    Measured(u64),
    Estimated(u64),
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "kind")]
pub enum Placement {
    Same {
        worker: String,
    },
    Moved {
        from: String,
        to: String,
        target_had_cache: Option<bool>,
    },
    /// value-mode workflows always run entirely in-process -- never through the worker pool --
    /// so there is structurally no placement decision here, not merely a missing one.
    NotDistributed,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Transport {
    InProcess,
    DurableArtifactHandoff,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "kind")]
pub enum Durability {
    Declared { required: bool },
    AutoResolved { required: bool, profile_id: String },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Cost {
    pub recompute_us: Metric,
    pub checkpoint_us: Metric,
    pub checkpoint_bytes: Metric,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ExplainEntry {
    pub step: String,
    pub placement: Placement,
    pub transport: Transport,
    pub durability: Durability,
    pub cost: Cost,
    pub durability_reason: Option<String>,
    pub placement_reason: Option<String>,
}

/// builds one entry per edge (consecutive step pair) from data the run already persisted --
/// `cell`'s `ComponentInspection`s (journaled durability plan + checkpoint facts) and `history`
/// (the control plane's own `Assigned` events). Never calls `durability_plan::decide` or
/// `worker::decide_placement`: nothing here is a second opinion on what was already decided.
pub fn explain_cell(cell: &CellInspection, history: &[RunEvent]) -> Vec<ExplainEntry> {
    let assignments: Vec<(&str, &AssignmentReason)> = history
        .iter()
        .filter_map(|event| match event {
            RunEvent::Assigned { worker, reason, .. } => Some((worker.as_str(), reason)),
            _ => None,
        })
        .collect();

    let mut entries = Vec::new();
    let mut group = 0usize;
    let mut current_worker = assignments.first().map(|&(worker, _)| worker);

    for pair in cell.components.windows(2) {
        let from = &pair[0];
        let to = &pair[1];
        let boundary = from.durable_after == Some(true);

        let (placement, placement_reason, transport) = if boundary {
            group += 1;
            match assignments.get(group) {
                Some(&(worker, reason)) => {
                    let label = transition_label(current_worker, worker, reason);
                    let placement = match current_worker {
                        Some(previous) if previous != worker => Placement::Moved {
                            from: previous.to_owned(),
                            to: worker.to_owned(),
                            target_had_cache: target_had_cache(reason),
                        },
                        _ => Placement::Same {
                            worker: worker.to_owned(),
                        },
                    };
                    current_worker = Some(worker);
                    (placement, Some(label), Transport::DurableArtifactHandoff)
                }
                None => (Placement::Unknown, None, Transport::DurableArtifactHandoff),
            }
        } else {
            let placement = match current_worker {
                Some(worker) => Placement::Same {
                    worker: worker.to_owned(),
                },
                None => Placement::Unknown,
            };
            (placement, None, Transport::InProcess)
        };

        entries.push(ExplainEntry {
            step: format!("{} -> {}", from.name, to.name),
            placement,
            transport,
            durability: durability_for(from),
            cost: cost_for(from),
            durability_reason: from.durability_reason.clone(),
            placement_reason,
        });
    }
    entries
}

/// value-mode workflows never leave the process, so there's no assignment history to consult --
/// every edge is `NotDistributed`/`InProcess`, and only durability/cost/reason apply.
pub fn explain_value_cell(cell: &ValueRunInspection) -> Vec<ExplainEntry> {
    cell.components
        .windows(2)
        .map(|pair| {
            let from = &pair[0];
            let to = &pair[1];
            ExplainEntry {
                step: format!("{} -> {}", from.name, to.name),
                placement: Placement::NotDistributed,
                transport: Transport::InProcess,
                durability: durability_for_value(from),
                cost: cost_for_value(from),
                durability_reason: from.durability_reason.clone(),
                placement_reason: None,
            }
        })
        .collect()
}

fn durability_for_value(component: &ValueComponentInspection) -> Durability {
    let required = component.durable_after.unwrap_or(false);
    match &component.planner_profile_id {
        Some(profile_id) => Durability::AutoResolved {
            required,
            profile_id: profile_id.clone(),
        },
        None => Durability::Declared { required },
    }
}

fn cost_for_value(component: &ValueComponentInspection) -> Cost {
    let estimate = |value: Option<u64>| value.map_or(Metric::Unknown, Metric::Estimated);
    let recompute_us = if component.durable_after == Some(false) {
        component.duration_us.map_or_else(
            || estimate(component.planner_recompute_us),
            Metric::Measured,
        )
    } else {
        estimate(component.planner_recompute_us)
    };
    Cost {
        recompute_us,
        checkpoint_us: component.checkpoint_duration_us.map_or_else(
            || estimate(component.planner_checkpoint_us),
            Metric::Measured,
        ),
        checkpoint_bytes: component.checkpoint_bytes.map_or_else(
            || estimate(component.planner_checkpoint_bytes),
            Metric::Measured,
        ),
    }
}

fn durability_for(component: &ComponentInspection) -> Durability {
    let required = component.durable_after.unwrap_or(false);
    match &component.planner_profile_id {
        Some(profile_id) => Durability::AutoResolved {
            required,
            profile_id: profile_id.clone(),
        },
        None => Durability::Declared { required },
    }
}

/// journaled planner numbers are the profile the planner compared to reach its decision -- a
/// real measurement, but from an earlier profiling run, used predictively for this edge. A
/// declared (non-auto) edge, or a journal written before schema v9, has none: `Unknown`, not `0`.
fn cost_for(component: &ComponentInspection) -> Cost {
    let estimate = |value: Option<u64>| value.map_or(Metric::Unknown, Metric::Estimated);
    // an ephemeral edge actually recomputes every run -- its own `duration_us` is a real
    // measurement of that recompute, more authoritative than the profile estimate that led to
    // the decision. A required edge never recomputes, so only the estimate is available.
    let recompute_us = if component.durable_after == Some(false) {
        component.duration_us.map_or_else(
            || estimate(component.planner_recompute_us),
            Metric::Measured,
        )
    } else {
        estimate(component.planner_recompute_us)
    };
    Cost {
        recompute_us,
        checkpoint_us: component.checkpoint_duration_us.map_or_else(
            || estimate(component.planner_checkpoint_us),
            Metric::Measured,
        ),
        checkpoint_bytes: component.checkpoint_bytes.map_or_else(
            || estimate(component.planner_checkpoint_bytes),
            Metric::Measured,
        ),
    }
}

fn target_had_cache(reason: &AssignmentReason) -> Option<bool> {
    match reason {
        AssignmentReason::ReassignedAfterGroupYield { target_had_cache } => Some(*target_had_cache),
        _ => None,
    }
}

/// the plain-language label for one real worker assignment -- the one place this reasoning is
/// spelled out, shared by `kairo explain`, the TUI detail view, and `kairo inspect`'s placement
/// section (`crates/cli/src/inspection/live.rs::print_placement`).
pub fn transition_label(previous: Option<&str>, worker: &str, reason: &AssignmentReason) -> String {
    match reason {
        AssignmentReason::Initial => format!("{worker} · initial assignment"),
        AssignmentReason::ReassignedAfterLeaseExpiry => {
            format!("{worker} · worker lost, reassigned")
        }
        AssignmentReason::ResumedAfterWait => format!("{worker} · resumed after wait"),
        AssignmentReason::ResumedAfterRestart => {
            format!("{worker} · resumed after control-plane restart")
        }
        AssignmentReason::ReassignedAfterGroupYield { target_had_cache } => {
            let from = previous.unwrap_or(worker);
            let cache = if *target_had_cache {
                " · target had cache"
            } else {
                ""
            };
            if from == worker {
                format!("{worker} \u{2192} {worker} · reassigned after durable boundary{cache}")
            } else {
                format!("{from} \u{2192} {worker} · moved after durable boundary{cache}")
            }
        }
    }
}
