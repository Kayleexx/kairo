#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::{
    fs,
    path::PathBuf,
    process::{self, Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
};

fn kairo() -> Command {
    Command::new(env!("CARGO_BIN_EXE_kairo"))
}

fn temp_dir(name: &str) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("kairo-{name}-{}-{sequence}", process::id()));
    fs::create_dir(&path).expect("fixture directory should be created");
    path
}

/// `kairo new` must never hang waiting for input it can't get, and must point at the real
/// scriptable equivalents instead of just failing generically.
#[test]
fn non_interactive_kairo_new_fails_immediately_with_a_scriptable_hint() {
    let directory = temp_dir("new-non-interactive");

    let output = kairo()
        .current_dir(&directory)
        .arg("new")
        .stdin(Stdio::null())
        .output()
        .expect("kairo new should run and return, not hang");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("interactive terminal"),
        "expected a clear non-interactive error, got: {stderr}"
    );
    assert!(
        stderr.contains("kairo workflow new") && stderr.contains("kairo workflow create"),
        "expected the scriptable equivalents to be named: {stderr}"
    );

    let _ = fs::remove_dir_all(directory);
}

/// the old subcommand-only entry point (`kairo new workflow ...`) is preserved unchanged as the
/// scriptable/legacy alternative to the new bare `kairo new`.
#[test]
fn legacy_new_workflow_subcommand_still_works() {
    let directory = temp_dir("new-workflow-legacy");
    let component =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../demos/basic/multiply-by-nine.wat");

    let created = kairo()
        .current_dir(&directory)
        .args(["new", "workflow", "legacy-flow", "--component"])
        .arg(&component)
        .args(["--input", "5"])
        .output()
        .expect("kairo new workflow should run");
    assert!(created.status.success(), "{created:?}");
    assert!(directory.join("legacy-flow.yaml").is_file());

    let _ = fs::remove_dir_all(directory);
}
