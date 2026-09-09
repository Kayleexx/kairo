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
        .args(["run", "guided.yaml"])
        .current_dir(&cwd)
        .output()
        .expect("generated workflow should run");
    assert!(run.status.success());
    assert_eq!(run.stdout, b"27\n");
    let _ = fs::remove_dir_all(cwd);
}

fn kairo() -> Command {
    Command::new(env!("CARGO_BIN_EXE_kairo"))
}
