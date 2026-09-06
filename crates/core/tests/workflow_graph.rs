#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::{fs, path::Path, process};

use kairo_core::{Workflow, WorkflowError};

fn parse(source: &str) -> Result<Workflow, WorkflowError> {
    Workflow::parse(source, Path::new("/workflows"), 256)
}

#[test]
fn parses_and_orders_a_linear_workflow() {
    let workflow = parse(
        r#"
workflow: example
input: 7
steps:
  - name: third
    component: third.wasm
  - name: first
    component: first.wasm
  - name: second
    component: second.wasm
edges:
  - from: first
    to: second
  - from: second
    to: third
"#,
    )
    .expect("workflow should parse");

    let names: Vec<_> = workflow
        .steps()
        .iter()
        .map(|step| step.id.as_str())
        .collect();
    assert_eq!(names, ["first", "second", "third"]);
    assert_eq!(workflow.input(), 7);
    assert_eq!(
        workflow.steps()[0].component,
        Path::new("/workflows/first.wasm")
    );
}

#[test]
fn rejects_cycles() {
    let error = parse(
        r#"
workflow: cycle
input: 0
steps:
  - { name: first, component: first.wasm }
  - { name: second, component: second.wasm }
edges:
  - { from: first, to: second }
  - { from: second, to: first }
"#,
    )
    .expect_err("cycle should fail");

    assert!(matches!(error, WorkflowError::Cycle));
}

#[test]
fn rejects_disconnected_steps() {
    let error = parse(
        r#"
workflow: disconnected
input: 0
steps:
  - { name: first, component: first.wasm }
  - { name: second, component: second.wasm }
edges: []
"#,
    )
    .expect_err("disconnected steps should fail");

    assert!(matches!(error, WorkflowError::Disconnected));
}

#[test]
fn rejects_branching_until_outputs_have_graph_semantics() {
    let error = parse(
        r#"
workflow: branch
input: 0
steps:
  - { name: first, component: first.wasm }
  - { name: second, component: second.wasm }
  - { name: third, component: third.wasm }
edges:
  - { from: first, to: second }
  - { from: first, to: third }
"#,
    )
    .expect_err("branch should fail");

    assert!(matches!(error, WorkflowError::MultipleOutputs { .. }));
}

#[test]
fn rejects_unknown_fields() {
    let error = parse(
        r#"
workflow: typo
input: 0
stepz: []
steps: []
edges: []
"#,
    )
    .expect_err("unknown field should fail");

    assert!(matches!(error, WorkflowError::InvalidYaml { .. }));
}

#[test]
fn rejects_duplicate_step_names() {
    let error = parse(
        r#"
workflow: duplicate
input: 0
steps:
  - { name: same, component: first.wasm }
  - { name: same, component: second.wasm }
edges: []
"#,
    )
    .expect_err("duplicate step should fail");

    assert!(matches!(error, WorkflowError::DuplicateStep { .. }));
}

#[test]
fn rejects_edges_to_unknown_steps() {
    let error = parse(
        r#"
workflow: unknown
input: 0
steps:
  - { name: first, component: first.wasm }
  - { name: second, component: second.wasm }
edges:
  - { from: first, to: missing }
"#,
    )
    .expect_err("unknown edge target should fail");

    assert!(matches!(error, WorkflowError::UnknownStep { .. }));
}

#[test]
fn rejects_empty_edge_steps_with_context() {
    let error = parse(
        r#"
workflow: empty-edge
input: 0
steps:
  - { name: first, component: first.wasm }
edges:
  - { from: "", to: first }
"#,
    )
    .expect_err("empty edge step should fail");

    assert!(matches!(
        error,
        WorkflowError::EmptyEdgeStep {
            index: 0,
            endpoint: "from"
        }
    ));
}

#[test]
fn rejects_too_many_steps() {
    let error = Workflow::parse(
        "workflow: limited\ninput: 0\nsteps:\n  - { name: first, component: first.wasm }\n  - { name: second, component: second.wasm }\nedges:\n  - { from: first, to: second }\n",
        Path::new("/workflows"),
        1,
    )
    .expect_err("step limit should be enforced");

    assert!(matches!(
        error,
        WorkflowError::TooManySteps {
            steps: 2,
            max_steps: 1
        }
    ));
}

#[test]
fn rejects_oversized_workflow_files() {
    let path = std::env::temp_dir().join(format!("kairo-workflow-{}.yaml", process::id()));
    fs::write(&path, "workflow: too-large").expect("fixture should be written");

    let result = Workflow::load(&path, 4, 256);
    let _ = fs::remove_file(path);

    assert!(matches!(result, Err(WorkflowError::TooLarge { .. })));
}
