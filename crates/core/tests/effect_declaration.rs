#![allow(clippy::expect_used)]

use std::path::Path;

use kairo_core::{Workflow, WorkflowError};

const STEPS: &str = "steps:\n  - name: apply\n    component: apply.wasm\nedges: []\n";

#[test]
fn parses_a_bounded_effect_operation() {
    let source =
        format!("workflow: effects\ninput: 7\neffect:\n  operation: record-order\n{STEPS}");
    let workflow = Workflow::parse(&source, Path::new("."), 8).expect("effect should parse");
    assert_eq!(
        workflow.effect().expect("effect should exist").operation(),
        "record-order"
    );
}

#[test]
fn rejects_an_unsafe_effect_operation() {
    let source =
        format!("workflow: effects\ninput: 7\neffect:\n  operation: http://remote\n{STEPS}");
    assert!(matches!(
        Workflow::parse(&source, Path::new("."), 8),
        Err(WorkflowError::InvalidEffect)
    ));
}
