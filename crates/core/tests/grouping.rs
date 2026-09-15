#![allow(clippy::expect_used)]

use std::{collections::BTreeMap, path::Path};

use kairo_core::{Workflow, plan_groups};

fn linear_workflow(source: &str) -> Workflow {
    Workflow::parse(source, Path::new("."), 8).expect("workflow should parse")
}

#[test]
fn a_workflow_with_no_required_edges_is_one_group() {
    let workflow = linear_workflow(
        "workflow: chain\ninput: 1\nsteps:\n  - name: a\n    component: a.wat\n  - name: b\n    component: b.wat\n  - name: c\n    component: c.wat\nedges:\n  - from: a\n    to: b\n  - from: b\n    to: c\n",
    );

    let groups = plan_groups(&workflow, &BTreeMap::new());

    assert_eq!(
        groups,
        vec![kairo_core::GroupSpan {
            start_index: 0,
            end_index: 2
        }]
    );
}

#[test]
fn a_required_edge_becomes_a_group_boundary() {
    let workflow = linear_workflow(
        "workflow: chain\ninput: 1\nsteps:\n  - name: a\n    component: a.wat\n  - name: b\n    component: b.wat\n  - name: c\n    component: c.wat\nedges:\n  - from: a\n    to: b\n    durability: required\n  - from: b\n    to: c\n",
    );
    let resolved = BTreeMap::from([(0, true)]);

    let groups = plan_groups(&workflow, &resolved);

    assert_eq!(
        groups,
        vec![
            kairo_core::GroupSpan {
                start_index: 0,
                end_index: 0
            },
            kairo_core::GroupSpan {
                start_index: 1,
                end_index: 2
            },
        ]
    );
}

#[test]
fn a_resolved_auto_edge_becomes_a_group_boundary_the_same_as_an_explicit_required_edge() {
    let workflow = linear_workflow(
        "workflow: chain\ninput: 1\nsteps:\n  - name: a\n    component: a.wat\n  - name: b\n    component: b.wat\n  - name: c\n    component: c.wat\nedges:\n  - from: a\n    to: b\n    durability: auto\n  - from: b\n    to: c\n    durability: required\n",
    );
    // the auto edge at index 0 resolved to required (e.g. via a Phase 14 profile) -- the
    // partition math treats it identically to the explicit required edge at index 1.
    let resolved = BTreeMap::from([(0, true), (1, true)]);

    let groups = plan_groups(&workflow, &resolved);

    assert_eq!(
        groups,
        vec![
            kairo_core::GroupSpan {
                start_index: 0,
                end_index: 0
            },
            kairo_core::GroupSpan {
                start_index: 1,
                end_index: 1
            },
            kairo_core::GroupSpan {
                start_index: 2,
                end_index: 2
            },
        ]
    );
}

#[test]
fn an_auto_edge_not_yet_resolved_is_never_treated_as_a_boundary() {
    let workflow = linear_workflow(
        "workflow: chain\ninput: 1\nsteps:\n  - name: a\n    component: a.wat\n  - name: b\n    component: b.wat\nedges:\n  - from: a\n    to: b\n    durability: auto\n",
    );

    // an absent entry (never resolved / unknown) must never be assumed required.
    let groups = plan_groups(&workflow, &BTreeMap::new());

    assert_eq!(
        groups,
        vec![kairo_core::GroupSpan {
            start_index: 0,
            end_index: 1
        }]
    );
}

#[test]
fn scalar_workflow_with_no_effect_or_wait_is_groupable() {
    let workflow = linear_workflow(
        "workflow: chain\ninput: 1\nsteps:\n  - name: a\n    component: a.wat\n  - name: b\n    component: b.wat\nedges:\n  - from: a\n    to: b\n    durability: required\n",
    );

    assert!(workflow.is_groupable());
}

#[test]
fn a_stream_workflow_is_never_groupable() {
    let workflow = linear_workflow(
        "workflow: stream\nmode: stream\ninput: input.bin\nsteps:\n  - name: a\n    component: a.wat\n  - name: b\n    component: b.wat\nedges:\n  - from: a\n    to: b\n    durability: auto\n",
    );

    assert!(!workflow.is_groupable());
}

#[test]
fn a_workflow_with_an_effect_boundary_is_never_groupable() {
    let workflow = linear_workflow(
        "workflow: chain\ninput: 1\nsteps:\n  - name: a\n    component: a.wat\n  - name: b\n    component: b.wat\nedges:\n  - from: a\n    to: b\n    durability: required\neffect:\n  operation: charge\n  after: a\n",
    );

    assert!(!workflow.is_groupable());
}

#[test]
fn a_workflow_with_a_wait_boundary_is_never_groupable() {
    let workflow = linear_workflow(
        "workflow: chain\ninput: 1\nsteps:\n  - name: a\n    component: a.wat\n  - name: b\n    component: b.wat\nedges:\n  - from: a\n    to: b\n    durability: required\nwait:\n  signal: continue\n  after: a\n",
    );

    assert!(!workflow.is_groupable());
}
