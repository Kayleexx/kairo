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
    assert!(stdout.contains("Usage: kairo [OPTIONS] [COMMAND]"));
    assert!(stdout.contains("component"));
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
fn rejects_a_missing_component() {
    let output = kairo()
        .args(["component", "check", "does-not-exist.wasm"])
        .output()
        .expect("kairo should start");

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).expect("error should be UTF-8");
    assert!(stderr.contains("error: load WebAssembly Component"));
    assert!(stderr.contains("does-not-exist.wasm"));
}
