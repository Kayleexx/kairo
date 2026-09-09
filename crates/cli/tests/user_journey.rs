#![allow(clippy::expect_used)]

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

#[test]
fn initializes_and_runs_a_discovered_workflow() {
    let directory = std::env::temp_dir().join(format!("kairo-journey-{}", std::process::id()));
    let _ = fs::remove_dir_all(&directory);
    fs::create_dir_all(directory.join("demos/checkout")).expect("fixture directory should exist");
    copy_demo(&directory, "workflow.yaml");
    for name in [
        "apply-discount.wat",
        "add-tax.wat",
        "add-handling.wat",
        "add-shipping.wat",
    ] {
        copy_demo(&directory, name);
    }
    let config = directory.join("config.toml");
    let initialized = command(&directory, &config)
        .arg("init")
        .output()
        .expect("init should run");
    assert!(initialized.status.success());
    assert!(String::from_utf8_lossy(&initialized.stdout).contains("ready     yes"));

    let run = command(&directory, &config)
        .args(["run", "checkout-settlement"])
        .output()
        .expect("discovered workflow should run");
    assert!(run.status.success());
    assert_eq!(run.stdout, b"3207\n");

    let inspected = command(&directory, &config)
        .args(["inspect", "checkout-settlement"])
        .output()
        .expect("workflow should be inspectable");
    assert!(inspected.status.success());

    let doctor = command(&directory, &config)
        .args(["doctor", "--json"])
        .output()
        .expect("doctor should run");
    assert!(doctor.status.success());
    assert_eq!(doctor.stdout, b"{\"project\":true,\"healthy\":true}\n");

    let watched = command(&directory, &config)
        .args(["run", "checkout-settlement", "--watch"])
        .output()
        .expect("watched workflow should run");
    assert!(watched.status.success());
    assert_eq!(watched.stdout, b"3207\n");
    let _ = fs::remove_dir_all(directory);
}

#[cfg(unix)]
#[test]
fn starts_and_stops_with_the_short_runtime_commands() {
    let directory = std::env::temp_dir().join(format!("kairo-up-{}", std::process::id()));
    let _ = fs::remove_dir_all(&directory);
    fs::create_dir(&directory).expect("fixture directory should exist");
    let config = directory.join("config.toml");
    assert!(
        command(&directory, &config)
            .arg("init")
            .output()
            .expect("init should run")
            .status
            .success()
    );
    fs::write(
        directory.join(".kairo/config.toml"),
        "[runtime]\nworkers = 1\n",
    )
    .expect("project defaults should be writable");
    assert!(
        command(&directory, &config)
            .arg("up")
            .output()
            .expect("up should run")
            .status
            .success()
    );
    let workers = command(&directory, &config)
        .arg("workers")
        .output()
        .expect("workers should run");
    assert!(String::from_utf8_lossy(&workers.stdout).contains("workers · 1"));
    assert!(
        command(&directory, &config)
            .arg("down")
            .output()
            .expect("down should run")
            .status
            .success()
    );
    let _ = fs::remove_dir_all(directory);
}

fn copy_demo(directory: &Path, name: &str) {
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../demos/checkout")
        .join(name);
    fs::copy(source, directory.join("demos/checkout").join(name)).expect("demo should copy");
}

fn command(directory: &Path, config: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_kairo"));
    command.current_dir(directory).env("KAIRO_CONFIG", config);
    command
}
