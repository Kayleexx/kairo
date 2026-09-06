#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::{path::PathBuf, process::Command};

fn kairo() -> Command {
    Command::new(env!("CARGO_BIN_EXE_kairo"))
}

#[test]
fn shows_help_without_arguments() {
    let output = kairo().output().expect("kairo should start");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8(output.stdout).expect("help should be UTF-8");
    assert!(stdout.contains("Run reliable workflows with WebAssembly Components"));
    assert!(stdout.contains("Usage: kairo [OPTIONS] [COMMAND]"));
    assert!(stdout.contains("  run  "));
    assert!(stdout.contains("  check"));
    assert!(!stdout.contains("run-component"));
    assert!(!stdout.contains('\u{1b}'));
}

#[test]
fn shows_plain_help_when_output_is_piped() {
    let output = kairo().arg("--help").output().expect("kairo should start");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8(output.stdout).expect("help should be UTF-8");
    assert!(stdout.contains("Usage: kairo [OPTIONS] [COMMAND]"));
    assert!(!stdout.contains('\u{1b}'));
}

#[test]
fn checks_a_valid_component() {
    let component =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../components/probe/component.wat");
    let output = kairo()
        .arg("check")
        .arg(&component)
        .output()
        .expect("kairo should start");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8(output.stdout).expect("output should be UTF-8");
    assert!(stdout.contains("valid component"));
    assert!(stdout.contains("sha256:"));
}

#[test]
fn runs_a_component() {
    let component =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../components/probe/component.wat");
    let output = kairo()
        .arg("run")
        .arg(&component)
        .args(["--input", "21"])
        .output()
        .expect("kairo should start");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert_eq!(output.stdout, b"42\n");
}

#[test]
fn keeps_the_hidden_run_component_command_compatible() {
    let component =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../components/probe/component.wat");
    let output = kairo()
        .arg("run-component")
        .arg(&component)
        .args(["--input", "21"])
        .output()
        .expect("kairo should start");

    assert!(output.status.success());
    assert_eq!(output.stdout, b"42\n");
}

#[test]
fn keeps_the_hidden_component_check_command_compatible() {
    let component =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../components/probe/component.wat");
    let output = kairo()
        .args(["component", "check"])
        .arg(&component)
        .output()
        .expect("kairo should start");

    assert!(output.status.success());
    assert!(output.stdout.starts_with(b"valid component"));
}

#[test]
fn runs_a_local_workflow() {
    let workflow =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../demos/basic/workflow.yaml");
    let output = kairo()
        .arg("run")
        .arg(&workflow)
        .output()
        .expect("kairo should start");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert_eq!(output.stdout, b"39\n");
}

#[test]
fn runs_workflow_yaml_by_default() {
    let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../demos/basic");
    let output = kairo()
        .arg("run")
        .current_dir(directory)
        .output()
        .expect("kairo should start");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert_eq!(output.stdout, b"39\n");
}

#[test]
fn checks_a_workflow_and_its_component_interfaces() {
    let workflow =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../demos/basic/workflow.yaml");
    let output = kairo()
        .arg("check")
        .arg(&workflow)
        .output()
        .expect("kairo should start");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert!(output.stdout.starts_with(b"valid workflow"));
    assert!(output.stdout.ends_with(b"3 components\n"));
}

#[test]
fn reports_real_execution_diagnostics() {
    let component =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../components/probe/component.wat");
    let output = kairo()
        .args(["run", "--verbose"])
        .arg(&component)
        .args(["--input", "21"])
        .env_remove("NO_COLOR")
        .env("CARGO_TERM_COLOR", "always")
        .output()
        .expect("kairo should start");

    assert!(output.status.success());
    assert_eq!(output.stdout, b"42\n");
    let stderr = String::from_utf8(output.stderr).expect("diagnostics should be UTF-8");
    assert!(stderr.contains("component executed"));
    assert!(stderr.contains("duration_us="));
    assert!(stderr.contains("hash=sha256:"));
    assert!(!stderr.contains('\u{1b}'));
}

#[test]
fn grants_console_only_when_requested() {
    let component =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../components/runtime/console.wat");
    let denied = kairo()
        .arg("run")
        .arg(&component)
        .args(["--input", "21"])
        .output()
        .expect("kairo should start");

    assert!(!denied.status.success());
    assert!(denied.stdout.is_empty());
    assert!(String::from_utf8_lossy(&denied.stderr).contains("console"));

    let granted = kairo()
        .arg("run")
        .arg(&component)
        .args(["--input", "21", "--allow-console"])
        .output()
        .expect("kairo should start");

    assert!(granted.status.success());
    assert_eq!(granted.stdout, b"42\n");
    assert_eq!(granted.stderr, b"guest: 21\n");
}

#[test]
fn rejects_invalid_component_input() {
    let component =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../components/probe/component.wat");
    let output = kairo()
        .arg("run")
        .arg(component)
        .args(["--input", "nope"])
        .output()
        .expect("kairo should start");

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("invalid value"));
}

#[test]
fn rejects_a_missing_component() {
    let output = kairo()
        .args(["check", "does-not-exist.wasm"])
        .output()
        .expect("kairo should start");

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).expect("error should be UTF-8");
    assert!(stderr.contains("error: failed to open component"));
    assert!(stderr.contains("does-not-exist.wasm"));
    assert!(stderr.contains("caused by:"));
    assert!(!stderr.contains('\u{1b}'));
}
