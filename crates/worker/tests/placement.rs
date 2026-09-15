use std::path::{Path, PathBuf};

use kairo_worker::{Placement, WorkerFacts, decide_placement, group_state_path};

fn worker(id: &str, healthy: bool, idle: bool, recent_for_shape: bool) -> WorkerFacts<'_> {
    WorkerFacts {
        id,
        healthy,
        idle,
        recent_for_shape,
    }
}

#[test]
fn stays_when_storage_is_not_shared() {
    let candidates = [worker("worker-2", true, true, false)];
    let placement = decide_placement("worker-1", false, Some(10), 1_000_000, &candidates);
    assert_eq!(placement, Placement::Stay);
}

#[test]
fn stays_when_no_other_worker_is_healthy_and_idle() {
    let candidates = [worker("worker-2", true, false, false)];
    let placement = decide_placement("worker-1", true, Some(10), 1_000_000, &candidates);
    assert_eq!(placement, Placement::Stay);
}

#[test]
fn stays_when_the_edge_bytes_are_unknown() {
    // unknown must never be treated as cheap -- staying local is the safe default.
    let candidates = [worker("worker-2", true, true, false)];
    let placement = decide_placement("worker-1", true, None, 1_000_000, &candidates);
    assert_eq!(placement, Placement::Stay);
}

#[test]
fn stays_when_the_edge_is_heavy() {
    let candidates = [worker("worker-2", true, true, false)];
    let placement = decide_placement("worker-1", true, Some(2_000_000), 1_000_000, &candidates);
    assert_eq!(placement, Placement::Stay);
}

#[test]
fn moves_to_the_idle_capable_worker_for_a_light_known_edge() {
    let candidates = [worker("worker-2", true, true, false)];
    let placement = decide_placement("worker-1", true, Some(10), 1_000_000, &candidates);
    assert_eq!(
        placement,
        Placement::Move {
            target: "worker-2".to_owned(),
            target_had_cache: false,
        }
    );
}

#[test]
fn prefers_the_cache_warm_candidate_over_a_cold_one() {
    let candidates = [
        worker("worker-2", true, true, false),
        worker("worker-3", true, true, true),
    ];
    let placement = decide_placement("worker-1", true, Some(10), 1_000_000, &candidates);
    assert_eq!(
        placement,
        Placement::Move {
            target: "worker-3".to_owned(),
            target_had_cache: true,
        }
    );
}

#[test]
fn breaks_ties_deterministically_by_worker_id() {
    let candidates = [
        worker("worker-b", true, true, false),
        worker("worker-a", true, true, false),
    ];
    let placement = decide_placement("worker-1", true, Some(10), 1_000_000, &candidates);
    assert_eq!(
        placement,
        Placement::Move {
            target: "worker-a".to_owned(),
            target_had_cache: false,
        }
    );
}

#[test]
fn excludes_unhealthy_and_busy_candidates() {
    let candidates = [
        worker("worker-2", false, true, false),
        worker("worker-3", true, false, false),
    ];
    let placement = decide_placement("worker-1", true, Some(10), 1_000_000, &candidates);
    assert_eq!(placement, Placement::Stay);
}

#[test]
fn group_state_path_leaves_group_zero_on_the_original_path() {
    assert_eq!(
        group_state_path(Path::new(".kairo/run.db"), 0),
        PathBuf::from(".kairo/run.db")
    );
}

#[test]
fn group_state_path_is_distinct_and_deterministic_for_a_later_group() {
    let first = group_state_path(Path::new(".kairo/run.db"), 3);
    let second = group_state_path(Path::new(".kairo/run.db"), 3);
    assert_eq!(first, second);
    assert_ne!(first, PathBuf::from(".kairo/run.db"));
    assert_eq!(first, PathBuf::from(".kairo/run.group-3.db"));
}
