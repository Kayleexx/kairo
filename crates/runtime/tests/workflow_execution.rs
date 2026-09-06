#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::path::{Path, PathBuf};

use kairo_core::{Config, Workflow};
use kairo_runtime::{Runtime, RuntimeError};

fn repository_path(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(path)
}

fn single_step_workflow(name: &str, component: &str) -> Workflow {
    let component = repository_path(component);
    let source = format!(
        "workflow: test\ninput: 21\nsteps:\n  - name: {name}\n    component: {}\nedges: []\n",
        component.display()
    );
    Workflow::parse(&source, Path::new("."), 1).expect("single-step workflow should be valid")
}

#[tokio::test]
async fn runs_real_components_in_graph_order() {
    let runtime = Runtime::new(Config::default()).expect("runtime should initialize");
    let workflow = runtime
        .load_workflow(repository_path("demos/basic/workflow.yaml"))
        .expect("workflow should load");

    let result = runtime
        .run_workflow(&workflow)
        .await
        .expect("workflow should run");

    assert_eq!(result.output, 68);
}

#[test]
fn rejects_components_with_an_incompatible_interface() {
    let workflow = single_step_workflow("probe", "components/probe/component.wat");
    let runtime = Runtime::new(Config::default()).expect("runtime should initialize");

    let error = runtime
        .validate_workflow(&workflow)
        .expect_err("wrong interface should fail");

    assert!(matches!(
        error,
        RuntimeError::IncompatibleWorkflowComponent { .. }
    ));
}

#[tokio::test]
async fn applies_capabilities_to_workflow_steps() {
    let workflow = single_step_workflow("console", "components/runtime/console.wat");
    let denied = Runtime::new(Config::default())
        .expect("runtime should initialize")
        .run_workflow(&workflow)
        .await
        .expect_err("console should be denied by default");
    assert!(matches!(
        denied,
        RuntimeError::WorkflowStep { step, source }
            if step == "console" && matches!(*source, RuntimeError::Instantiate { .. })
    ));

    let runtime = Runtime::new(Config {
        allow_console: true,
        ..Config::default()
    })
    .expect("runtime should initialize");
    let result = runtime
        .run_workflow(&workflow)
        .await
        .expect("granted console should run");

    assert_eq!(result.output, 42);
}

#[tokio::test]
async fn preserves_fuel_errors_and_step_context() {
    let workflow = single_step_workflow("runaway", "components/runtime/runaway.wat");
    let runtime = Runtime::new(Config::default()).expect("runtime should initialize");
    let error = runtime
        .run_workflow(&workflow)
        .await
        .expect_err("runaway step should fail");

    assert!(matches!(
        error,
        RuntimeError::WorkflowStep { step, source }
            if step == "runaway" && matches!(*source, RuntimeError::FuelExhausted { .. })
    ));
}

#[tokio::test]
async fn preserves_memory_errors_and_step_context() {
    let workflow = single_step_workflow("memory", "components/runtime/memory-limit.wat");
    let runtime = Runtime::new(Config::default()).expect("runtime should initialize");
    let error = runtime
        .run_workflow(&workflow)
        .await
        .expect_err("memory-limited step should fail");

    assert!(matches!(
        error,
        RuntimeError::WorkflowStep { step, source }
            if step == "memory" && matches!(*source, RuntimeError::MemoryLimitExceeded { .. })
    ));
}
