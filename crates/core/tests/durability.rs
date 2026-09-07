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
fn rejects_required_stream_edges() {
    let error = Workflow::parse(
        "workflow: stream\nmode: stream\ninput: input.bin\nsteps:\n  - name: first\n    component: first.wat\n  - name: second\n    component: second.wat\nedges:\n  - from: first\n    to: second\n    durability: required\n",
        Path::new("."),
        2,
    )
    .expect_err("stream durability should be rejected");

    assert!(matches!(error, WorkflowError::StreamDurability));
}
