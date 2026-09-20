#![allow(clippy::expect_used)]

use std::path::Path;

use kairo_core::{Durability, Workflow, WorkflowError};

#[test]
fn defaults_edges_to_ephemeral() {
    let workflow = Workflow::parse(
        "workflow: example\ninput: 1\nsteps:\n  - name: first\n    component: first.wat\n  - name: second\n    component: second.wat\nedges:\n  - from: first\n    to: second\n",
        Path::new("."),
        2,
    )
    .expect("workflow should parse");

    assert_eq!(workflow.durability_after_step(0), Durability::Ephemeral);
    assert!(!workflow.requires_durable_artifacts());
}

#[test]
fn parses_auto_as_unresolved_without_requiring_durable_artifacts() {
    let workflow = Workflow::parse(
        "workflow: example\ninput: 1\nsteps:\n  - name: first\n    component: first.wat\n  - name: second\n    component: second.wat\nedges:\n  - from: first\n    to: second\n    durability: auto\n",
        Path::new("."),
        2,
    )
    .expect("workflow should parse");

    assert_eq!(workflow.durability_after_step(0), Durability::Auto);
    assert!(workflow.durability_after_step(0).is_unresolved());
    assert!(!workflow.requires_durable_artifacts());
}

#[test]
fn does_not_treat_auto_as_a_durable_boundary() {
    let error = Workflow::parse(
        "workflow: example\ninput: 1\nsteps:\n  - name: first\n    component: first.wat\n  - name: second\n    component: second.wat\nedges:\n  - from: first\n    to: second\n    durability: auto\nwait:\n  signal: continue\n  after: first\n",
        Path::new("."),
        2,
    )
    .expect_err("auto durability cannot satisfy a durable boundary");

    assert!(matches!(error, WorkflowError::InvalidBoundary { .. }));
}

#[test]
fn accepts_required_stream_edges() {
    Workflow::parse(
        "workflow: stream\nmode: stream\ninput: input.bin\nsteps:\n  - name: first\n    component: first.wat\n  - name: second\n    component: second.wat\nedges:\n  - from: first\n    to: second\n    durability: required\n",
        Path::new("."),
        2,
    )
    .expect("a required edge is a legal durability cut for a stream workflow");
}

#[test]
fn accepts_auto_stream_edges() {
    Workflow::parse(
        "workflow: stream\nmode: stream\ninput: input.bin\nsteps:\n  - name: first\n    component: first.wat\n  - name: second\n    component: second.wat\nedges:\n  - from: first\n    to: second\n    durability: auto\n",
        Path::new("."),
        2,
    )
    .expect("the stream planner resolves `durability: auto`");
}
