#![allow(clippy::expect_used)]

use std::path::Path;

use kairo_core::{Workflow, WorkflowError};

const WORKFLOW: &str = "workflow: approval\ninput: 7\nsteps:\n  - name: prepare\n    component: prepare.wasm\n  - name: finish\n    component: finish.wasm\nedges:\n  - from: prepare\n    to: finish\n    durability: required\n";

#[test]
fn accepts_a_wait_after_a_durable_component() {
    let source = format!("{WORKFLOW}wait:\n  signal: approved\n  after: prepare\n");
    let workflow = Workflow::parse(&source, Path::new("."), 8).expect("workflow should parse");
    assert_eq!(
        workflow
            .wait_after()
            .expect("wait boundary should exist")
            .as_str(),
        "prepare"
    );
}

#[test]
fn rejects_a_boundary_without_a_durable_edge() {
    let source = format!(
        "{}wait:\n  signal: approved\n  after: finish\n",
        WORKFLOW.replace("    durability: required\n", "")
    );
    assert!(matches!(
        Workflow::parse(&source, Path::new("."), 8),
        Err(WorkflowError::InvalidBoundary { kind: "wait", .. })
    ));
}

#[test]
fn rejects_wait_and_effect_at_the_same_boundary() {
    let source = format!(
        "{WORKFLOW}wait:\n  signal: approved\n  after: prepare\neffect:\n  operation: create-order\n  after: prepare\n"
    );
    assert!(matches!(
        Workflow::parse(&source, Path::new("."), 8),
        Err(WorkflowError::ConflictingBoundaries)
    ));
}
