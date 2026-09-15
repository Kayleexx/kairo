#![allow(clippy::expect_used)]

use std::{fs, path::PathBuf, process::Command};

#[test]
fn creates_and_runs_a_workflow_from_a_component() {
    let component =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../demos/basic/multiply-by-nine.wat");
    let cwd = std::env::temp_dir().join(format!("kairo-new-workflow-{}", std::process::id()));
    let _ = fs::remove_dir_all(&cwd);
    fs::create_dir(&cwd).expect("temp directory should be created");

    let created = kairo()
        .args(["new", "workflow", "multiply", "--component"])
        .arg(&component)
        .args(["--input", "21"])
        .current_dir(&cwd)
        .output()
        .expect("workflow should be created");
    assert!(created.status.success());
    assert!(String::from_utf8_lossy(&created.stdout).contains("kairo run multiply.yaml"));

    let run = kairo()
        .args(["run", "multiply.yaml"])
        .current_dir(&cwd)
        .output()
        .expect("generated workflow should run");
    assert!(run.status.success());
    assert_eq!(run.stdout, b"189\n");

    let _ = fs::remove_dir_all(cwd);
}

#[test]
fn creates_a_workflow_with_the_guided_creator_flags() {
    let component =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../demos/basic/multiply-by-nine.wat");
    let cwd = std::env::temp_dir().join(format!("kairo-guided-workflow-{}", std::process::id()));
    let _ = fs::remove_dir_all(&cwd);
    fs::create_dir(&cwd).expect("temp directory should be created");

    let created = kairo()
        .args(["workflow", "create", "--name", "guided", "--component"])
        .arg(&component)
        .args(["--input", "3"])
        .current_dir(&cwd)
        .output()
        .expect("workflow should be created");
    assert!(created.status.success());
    assert!(cwd.join("guided.yaml").is_file());

    let run = kairo()
        .args(["run", "guided"])
        .current_dir(&cwd)
        .output()
        .expect("generated workflow should run");
    assert!(run.status.success());
    assert_eq!(run.stdout, b"27\n");
    let _ = fs::remove_dir_all(cwd);
}

#[test]
fn creator_defaults_generated_edges_to_auto_durability() {
    let component =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../demos/basic/multiply-by-nine.wat");
    let second =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../demos/basic/divide-by-five.wat");
    let cwd = std::env::temp_dir().join(format!(
        "kairo-creator-default-durability-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&cwd);
    fs::create_dir(&cwd).expect("temp directory should be created");

    // no `--durability` flag at all -- a first-time author shouldn't have to know it exists.
    let created = kairo()
        .args(["workflow", "create", "--name", "defaulted", "--component"])
        .arg(&component)
        .args(["--component"])
        .arg(&second)
        .current_dir(&cwd)
        .output()
        .expect("workflow should be created");
    assert!(created.status.success());
    let source = fs::read_to_string(cwd.join("defaulted.yaml")).expect("workflow exists");
    assert!(source.contains("durability: auto"));
    let _ = fs::remove_dir_all(cwd);
}

#[test]
fn creator_writes_control_options() {
    let component =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../demos/basic/multiply-by-nine.wat");
    let second =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../demos/basic/divide-by-five.wat");
    let cwd = std::env::temp_dir().join(format!("kairo-creator-options-{}", std::process::id()));
    let _ = fs::remove_dir_all(&cwd);
    fs::create_dir(&cwd).expect("temp directory should be created");
    let created = kairo()
        .args([
            "workflow",
            "create",
            "--name",
            "durable-wait",
            "--component",
        ])
        .arg(&component)
        .args(["--component"])
        .arg(&second)
        .args([
            "--durability",
            "required",
            "--wait",
            "signal:approval.granted",
            "--effect",
            "record-order",
        ])
        .current_dir(&cwd)
        .output()
        .expect("workflow should be created");
    assert!(created.status.success());
    let source = fs::read_to_string(cwd.join("durable-wait.yaml")).expect("workflow exists");
    assert!(source.contains("durability: required"));
    assert!(source.contains("signal: \"approval.granted\""));
    assert!(source.contains("operation: \"record-order\""));
    let _ = fs::remove_dir_all(cwd);
}

#[test]
fn creator_accepts_auto_durability() {
    let component =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../demos/basic/multiply-by-nine.wat");
    let second =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../demos/basic/divide-by-five.wat");
    let cwd = std::env::temp_dir().join(format!("kairo-creator-auto-{}", std::process::id()));
    let _ = fs::remove_dir_all(&cwd);
    fs::create_dir(&cwd).expect("temp directory should be created");
    let created = kairo()
        .args(["workflow", "create", "--name", "auto-flow", "--component"])
        .arg(&component)
        .args(["--component"])
        .arg(&second)
        .args(["--durability", "auto"])
        .current_dir(&cwd)
        .output()
        .expect("workflow should be created");
    assert!(created.status.success(), "{created:?}");
    let source = fs::read_to_string(cwd.join("auto-flow.yaml")).expect("workflow exists");
    assert!(source.contains("durability: auto"));
    let _ = fs::remove_dir_all(cwd);
}

fn kairo() -> Command {
    Command::new(env!("CARGO_BIN_EXE_kairo"))
}
