#![allow(clippy::expect_used)]

use std::path::Path;

use kairo_core::{Workflow, WorkflowError, WorkflowResources};

fn workflow(resources: &str) -> Result<Workflow, WorkflowError> {
    Workflow::parse(
        &format!(
            "workflow: bounded\ninput: 1\n{resources}steps:\n  - name: step\n    component: step.wat\nedges: []\n"
        ),
        Path::new("."),
        1,
    )
}

#[test]
fn parses_explicit_workflow_resources() {
    let workflow = workflow("resources:\n  fuel: 1250000000\n  memory_bytes: 16777216\n")
        .expect("resources should parse");

    assert_eq!(
        workflow.resources(),
        Some(WorkflowResources {
            fuel: 1_250_000_000,
            memory_bytes: 16_777_216,
        })
    );
}

#[test]
fn leaves_resources_unspecified_for_compatibility() {
    assert_eq!(
        workflow("").expect("workflow should parse").resources(),
        None
    );
}

#[test]
fn rejects_zero_resource_limits() {
    for resources in [
        "resources:\n  fuel: 0\n  memory_bytes: 1\n",
        "resources:\n  fuel: 1\n  memory_bytes: 0\n",
    ] {
        assert!(matches!(
            workflow(resources),
            Err(WorkflowError::ZeroResources)
        ));
    }
}
