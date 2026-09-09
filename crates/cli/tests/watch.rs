#![allow(clippy::expect_used)]

use std::{path::PathBuf, process::Command};

fn kairo() -> Command {
    Command::new(env!("CARGO_BIN_EXE_kairo"))
}

#[test]
fn rejects_watch_for_a_component() {
    let component =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../components/probe/component.wat");
    let output = kairo()
        .args(["run", "--watch"])
        .arg(component)
        .output()
        .expect("kairo should start");

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--watch"));
}

#[test]
fn requires_watch_before_accepting_worker_count() {
    let workflow =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../demos/basic/workflow.yaml");
    let output = kairo()
        .args(["run", "--workers", "3"])
        .arg(workflow)
        .output()
        .expect("kairo should start");

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--watch"));
}
