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
    assert!(stdout.contains("component"));
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
        .args(["component", "check"])
        .arg(&component)
        .output()
        .expect("kairo should start");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8(output.stdout).expect("output should be UTF-8");
    assert!(stdout.contains("Component is valid:"));
}

#[test]
fn runs_a_component() {
    let component =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../components/probe/component.wat");
    let output = kairo()
        .arg("run-component")
        .arg(&component)
        .output()
        .expect("kairo should start");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert_eq!(output.stdout, b"42\n");
}

#[test]
fn reports_real_execution_diagnostics() {
    let component =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../components/probe/component.wat");
    let output = kairo()
        .args(["--verbose", "run-component"])
        .arg(&component)
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
fn rejects_a_missing_component() {
    let output = kairo()
        .args(["component", "check", "does-not-exist.wasm"])
        .output()
        .expect("kairo should start");

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).expect("error should be UTF-8");
    assert!(stderr.contains("error: open WebAssembly Component"));
    assert!(stderr.contains("does-not-exist.wasm"));
}
