#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::path::{Path, PathBuf};

use kairo_core::{Config, Workflow};
use kairo_runtime::{RuntimeError, detect_contract};

fn repository_path(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(path)
}

fn workflow_with_hash(component: &Path, hash: Option<&str>) -> Workflow {
    let hash_line = hash.map_or_else(String::new, |hash| format!("    hash: \"{hash}\"\n"));
    let source = format!(
        "workflow: test\ninput: 21\nsteps:\n  - name: step\n    component: {}\n{hash_line}edges: []\n",
        component.display()
    );
    Workflow::parse(&source, Path::new("."), 1).expect("workflow should be valid")
}

#[test]
fn a_correctly_pinned_hash_validates() {
    let component = repository_path("demos/basic/multiply-by-nine.wat");
    let contract = detect_contract(&component, Config::default())
        .expect("component should produce a real contract");
    let workflow = workflow_with_hash(&component, Some(&contract.hash.to_string()));
    let runtime = kairo_runtime::Runtime::new(Config::default()).expect("runtime should init");

    runtime
        .validate_workflow(&workflow)
        .expect("a matching pinned hash should validate");
}

#[test]
fn a_stale_pinned_hash_is_refused_not_silently_run() {
    let component = repository_path("demos/basic/multiply-by-nine.wat");
    let stale_hash = format!("sha256:{}", "0".repeat(64));
    let workflow = workflow_with_hash(&component, Some(&stale_hash));
    let runtime = kairo_runtime::Runtime::new(Config::default()).expect("runtime should init");

    let error = runtime
        .validate_workflow(&workflow)
        .expect_err("a stale pinned hash must never validate");
    assert!(
        matches!(error, RuntimeError::PinnedComponentMismatch { .. }),
        "{error:?}"
    );
}

#[test]
fn an_unpinned_workflow_still_validates_as_before() {
    let component = repository_path("demos/basic/multiply-by-nine.wat");
    let workflow = workflow_with_hash(&component, None);
    let runtime = kairo_runtime::Runtime::new(Config::default()).expect("runtime should init");

    runtime
        .validate_workflow(&workflow)
        .expect("an unpinned step is unaffected by the new check");
}
