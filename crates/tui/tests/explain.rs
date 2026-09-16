use kairo_control::{AssignmentReason, RunEvent};
use kairo_runtime::{
    CellInspection, CellStatus, ComponentInspection, ValueComponentInspection, ValueRunInspection,
    ValueRunStatus,
};
use kairo_tui::explain::{
    Durability, Metric, Placement, Transport, explain_cell, explain_value_cell,
};

fn component(index: usize, name: &str) -> ComponentInspection {
    ComponentInspection {
        index,
        name: name.to_owned(),
        hash: "sha256:test".to_owned(),
        input: 1,
        output: Some(1),
        duration_us: Some(100),
        durable_after: None,
        checkpoint: None,
        checkpoint_backend: None,
        checkpoint_bytes: None,
        checkpoint_duration_us: None,
        durability_reason: None,
        planner_profile_id: None,
        planner_recompute_us: None,
        planner_checkpoint_us: None,
        planner_checkpoint_bytes: None,
        planner_samples: None,
        attempts: 1,
    }
}

fn cell(components: Vec<ComponentInspection>) -> CellInspection {
    CellInspection {
        name: Some("test".to_owned()),
        input: 1,
        status: CellStatus::Completed { output: 1 },
        components,
        metadata_complete: true,
        recovery_duration_us: None,
    }
}

fn assigned(worker: &str, reason: AssignmentReason) -> RunEvent {
    RunEvent::Assigned {
        worker: worker.to_owned(),
        epoch: 0,
        at_ms: 0,
        reason,
    }
}

#[test]
fn cheap_recompute_ephemeral_edge_reports_no_checkpoint_and_an_estimated_recompute() {
    let mut from = component(0, "warm-up");
    from.durable_after = Some(false);
    from.duration_us = Some(91);
    from.durability_reason =
        Some("shape-a · recompute 91us <= 2x checkpoint 1168us (1 bytes, 1 sample(s))".to_owned());
    from.planner_profile_id = Some("shape-a".to_owned());
    from.planner_recompute_us = Some(91);
    from.planner_checkpoint_us = Some(1168);
    from.planner_checkpoint_bytes = Some(1);
    from.planner_samples = Some(1);
    let to = component(1, "finish");
    let entries = explain_cell(
        &cell(vec![from, to]),
        &[assigned("w1", AssignmentReason::Initial)],
    );

    assert_eq!(entries.len(), 1);
    let entry = &entries[0];
    assert_eq!(entry.transport, Transport::InProcess);
    assert!(matches!(
        entry.durability,
        Durability::AutoResolved {
            required: false,
            ..
        }
    ));
    // an ephemeral edge's own `duration_us` is a real measurement of this run's recompute.
    assert_eq!(entry.cost.recompute_us, Metric::Measured(91));
    assert_eq!(entry.cost.checkpoint_bytes, Metric::Estimated(1));
    assert!(entry.durability_reason.is_some());
}

#[test]
fn expensive_recompute_durable_edge_records_a_real_checkpoint_measurement() {
    let mut from = component(0, "slow-compute");
    from.durable_after = Some(true);
    from.checkpoint = Some("sha256:checkpoint".to_owned());
    from.checkpoint_bytes = Some(128);
    from.checkpoint_duration_us = Some(702);
    from.durability_reason = Some(
        "shape-b · recompute 4200us > 2x checkpoint 702us (128 bytes, 3 sample(s))".to_owned(),
    );
    from.planner_profile_id = Some("shape-b".to_owned());
    from.planner_recompute_us = Some(4200);
    from.planner_checkpoint_us = Some(702);
    from.planner_checkpoint_bytes = Some(128);
    from.planner_samples = Some(3);
    let to = component(1, "finish");
    let entries = explain_cell(
        &cell(vec![from, to]),
        &[assigned("w1", AssignmentReason::Initial)],
    );

    let entry = &entries[0];
    assert!(matches!(
        entry.durability,
        Durability::AutoResolved { required: true, .. }
    ));
    // this run actually checkpointed -- that's a measured fact, not the planner's estimate.
    assert_eq!(entry.cost.checkpoint_bytes, Metric::Measured(128));
    assert_eq!(entry.cost.checkpoint_us, Metric::Measured(702));
    assert_eq!(entry.cost.recompute_us, Metric::Estimated(4200));
}

#[test]
fn same_worker_placement_reports_no_move() {
    let mut from = component(0, "a");
    from.durable_after = Some(true);
    let to = component(1, "b");
    let entries = explain_cell(
        &cell(vec![from, to]),
        &[
            assigned("w1", AssignmentReason::Initial),
            assigned(
                "w1",
                AssignmentReason::ReassignedAfterGroupYield {
                    target_had_cache: true,
                },
            ),
        ],
    );

    assert_eq!(
        entries[0].placement,
        Placement::Same {
            worker: "w1".to_owned()
        }
    );
    assert_eq!(entries[0].transport, Transport::DurableArtifactHandoff);
    assert!(entries[0].placement_reason.is_some());
}

#[test]
fn cross_worker_handoff_reports_the_move_and_cache_hit() {
    let mut from = component(0, "a");
    from.durable_after = Some(true);
    let to = component(1, "b");
    let entries = explain_cell(
        &cell(vec![from, to]),
        &[
            assigned("w1", AssignmentReason::Initial),
            assigned(
                "w2",
                AssignmentReason::ReassignedAfterGroupYield {
                    target_had_cache: true,
                },
            ),
        ],
    );

    assert_eq!(
        entries[0].placement,
        Placement::Moved {
            from: "w1".to_owned(),
            to: "w2".to_owned(),
            target_had_cache: Some(true),
        }
    );
}

#[test]
fn declared_durability_has_no_planner_reason_or_estimate() {
    let mut from = component(0, "a");
    from.durable_after = Some(true);
    from.checkpoint_bytes = Some(64);
    let to = component(1, "b");
    let entries = explain_cell(
        &cell(vec![from, to]),
        &[assigned("w1", AssignmentReason::Initial)],
    );

    assert_eq!(
        entries[0].durability,
        Durability::Declared { required: true }
    );
    assert!(entries[0].durability_reason.is_none());
    assert_eq!(entries[0].cost.checkpoint_bytes, Metric::Measured(64));
    assert_eq!(entries[0].cost.recompute_us, Metric::Unknown);
}

#[test]
fn missing_assignment_history_reports_unknown_placement_never_a_guess() {
    let mut from = component(0, "a");
    from.durable_after = Some(true);
    let to = component(1, "b");
    let entries = explain_cell(&cell(vec![from, to]), &[]);

    assert_eq!(entries[0].placement, Placement::Unknown);
}

fn value_component(index: usize, name: &str) -> ValueComponentInspection {
    ValueComponentInspection {
        index,
        name: name.to_owned(),
        hash: "sha256:test".to_owned(),
        input_preview: "7".to_owned(),
        output_preview: Some("70000".to_owned()),
        duration_us: Some(40),
        durable_after: Some(false),
        checkpoint: None,
        checkpoint_backend: None,
        checkpoint_bytes: None,
        checkpoint_duration_us: None,
        durability_reason: Some("shape-a · recompute 40us <= 2x checkpoint 632us".to_owned()),
        planner_profile_id: Some("shape-a".to_owned()),
        planner_recompute_us: Some(40),
        planner_checkpoint_us: Some(632),
        planner_checkpoint_bytes: Some(5),
        planner_samples: Some(1),
        attempts: 1,
    }
}

fn value_cell(components: Vec<ValueComponentInspection>) -> ValueRunInspection {
    ValueRunInspection {
        name: Some("test".to_owned()),
        input_preview: "7".to_owned(),
        status: ValueRunStatus::Completed {
            output_preview: "23".to_owned(),
        },
        components,
        metadata_complete: true,
        recovery_duration_us: None,
    }
}

#[test]
fn value_mode_workflows_report_not_distributed_never_a_fabricated_placement() {
    let from = value_component(0, "expand-range");
    let to = value_component(1, "count-primes");
    let entries = explain_value_cell(&value_cell(vec![from, to]));

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].placement, Placement::NotDistributed);
    assert_eq!(entries[0].transport, Transport::InProcess);
    assert!(matches!(
        entries[0].durability,
        Durability::AutoResolved {
            required: false,
            ..
        }
    ));
    assert_eq!(entries[0].cost.recompute_us, Metric::Measured(40));
    assert_eq!(entries[0].cost.checkpoint_us, Metric::Estimated(632));
    assert!(entries[0].durability_reason.is_some());
}
