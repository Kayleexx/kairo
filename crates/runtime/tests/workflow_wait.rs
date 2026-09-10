#![allow(clippy::expect_used)]

use std::{fs, path::PathBuf};

use kairo_runtime::{
    DurableWait, complete_workflow_wait, inspect_workflow_wait, record_workflow_wait,
};

fn path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("kairo-{name}-{}.db", std::process::id()))
}

#[test]
fn records_and_completes_a_durable_signal_wait() {
    let path = path("workflow-wait");
    let _ = fs::remove_file(&path);
    let wait = DurableWait::Signal {
        name: "approval.granted".to_owned(),
    };
    let recorded = record_workflow_wait(&path, 1, &wait).expect("wait should be recorded");
    assert_eq!(recorded.wait, wait);
    assert!(!recorded.completed);

    complete_workflow_wait(&path, 1).expect("wait should complete");
    let completed = inspect_workflow_wait(&path)
        .expect("wait should be readable")
        .expect("wait should exist");
    assert!(completed.completed);
    let _ = fs::remove_file(path);
}

#[test]
fn rejects_a_different_wait_for_the_same_run() {
    let path = path("workflow-wait-conflict");
    let _ = fs::remove_file(&path);
    record_workflow_wait(&path, 0, &DurableWait::Timer { due_ms: 100 })
        .expect("first wait should be recorded");
    assert!(record_workflow_wait(&path, 0, &DurableWait::Timer { due_ms: 200 }).is_err());
    let _ = fs::remove_file(path);
}
