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
    single_step_workflow_with(name, component, 21, "")
}

fn single_step_workflow_with(name: &str, component: &str, input: u32, resources: &str) -> Workflow {
    let component = repository_path(component);
    let source = format!(
        "workflow: test\ninput: {input}\n{resources}steps:\n  - name: {name}\n    component: {}\nedges: []\n",
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

#[tokio::test]
async fn applies_explicit_workflow_fuel() {
    let workflow = single_step_workflow_with(
        "console",
        "components/runtime/console.wat",
        1,
        "resources:\n  fuel: 10000000\n  memory_bytes: 67108864\n",
    );
    let runtime = Runtime::new(Config {
        allow_console: true,
        execution_fuel: 100,
        ..Config::default()
    })
    .expect("runtime should initialize");

    let result = runtime
        .run_workflow(&workflow)
        .await
        .expect("workflow override should replace the default fuel budget");
    assert_eq!(result.output, 2);
}

#[tokio::test]
async fn applies_explicit_workflow_memory() {
    let workflow = single_step_workflow_with(
        "memory",
        "components/runtime/memory-limit.wat",
        1,
        "resources:\n  fuel: 10000000\n  memory_bytes: 83886080\n",
    );
    let result = Runtime::new(Config::default())
        .expect("runtime should initialize")
        .run_workflow(&workflow)
        .await
        .expect("workflow override should replace the default memory budget");

    assert_eq!(result.output, 0);
}

#[tokio::test]
async fn reports_the_explicit_memory_limit_on_exhaustion() {
    let workflow = single_step_workflow_with(
        "memory",
        "components/runtime/memory-limit.wat",
        1,
        "resources:\n  fuel: 10000000\n  memory_bytes: 16777216\n",
    );
    let error = Runtime::new(Config::default())
        .expect("runtime should initialize")
        .run_workflow(&workflow)
        .await
        .expect_err("workflow should exhaust its explicit memory limit");

    assert!(matches!(
        error,
        RuntimeError::WorkflowStep { source, .. }
            if matches!(
                *source,
                RuntimeError::MemoryLimitExceeded {
                    max_memory_bytes: 16_777_216,
                    ..
                }
            )
    ));
}

#[tokio::test]
async fn reports_the_explicit_fuel_limit_on_exhaustion() {
    let workflow = single_step_workflow_with(
        "console",
        "components/runtime/console.wat",
        1,
        "resources:\n  fuel: 1\n  memory_bytes: 67108864\n",
    );
    let error = Runtime::new(Config {
        allow_console: true,
        ..Config::default()
    })
    .expect("runtime should initialize")
    .run_workflow(&workflow)
    .await
    .expect_err("workflow should exhaust its explicit fuel limit");

    assert!(matches!(
        error,
        RuntimeError::WorkflowStep { source, .. }
            if matches!(*source, RuntimeError::FuelExhausted { fuel: 1, .. })
    ));
}

#[test]
fn rejects_workflow_resources_above_host_maxima() {
    let runtime = Runtime::new(Config::default()).expect("runtime should initialize");
    let excessive_fuel = single_step_workflow_with(
        "compute",
        "components/runtime/compute.wat",
        1,
        "resources:\n  fuel: 5000000001\n  memory_bytes: 67108864\n",
    );
    let excessive_memory = single_step_workflow_with(
        "compute",
        "components/runtime/compute.wat",
        1,
        "resources:\n  fuel: 10000000\n  memory_bytes: 268435457\n",
    );

    assert!(matches!(
        runtime.validate_workflow(&excessive_fuel),
        Err(RuntimeError::WorkflowFuelLimit { .. })
    ));
    assert!(matches!(
        runtime.validate_workflow(&excessive_memory),
        Err(RuntimeError::WorkflowMemoryLimit { .. })
    ));
}
