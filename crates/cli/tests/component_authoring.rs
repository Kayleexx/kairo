#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::{
    fs,
    path::PathBuf,
    process::{self, Command},
    sync::atomic::{AtomicU64, Ordering},
};

fn kairo() -> Command {
    Command::new(env!("CARGO_BIN_EXE_kairo"))
}

struct Directory(PathBuf);

impl Directory {
    fn new(name: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("kairo-{name}-{}-{sequence}", process::id()));
        fs::create_dir(&path).expect("fixture directory should be created");
        Self(path)
    }
}

impl Drop for Directory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn scaffolds_a_component_project_that_refuses_to_overwrite_itself() {
    let directory = Directory::new("component-new");

    let output = kairo()
        .current_dir(&directory.0)
        .args(["component", "new", "order-validate"])
        .output()
        .expect("kairo component new should run");
    assert!(output.status.success(), "{output:?}");

    let project = directory.0.join("components/order-validate");
    assert!(project.join("Cargo.toml").is_file());
    assert!(project.join("wit/workflow.wit").is_file());
    assert!(project.join("src/lib.rs").is_file());

    let manifest = fs::read_to_string(project.join("Cargo.toml")).expect("Cargo.toml should read");
    assert!(manifest.contains("name = \"order-validate\""));
    assert!(manifest.contains("wit-bindgen"));

    // scaffolding again must not silently clobber whatever the user has started writing.
    let repeat = kairo()
        .current_dir(&directory.0)
        .args(["component", "new", "order-validate"])
        .output()
        .expect("kairo component new should run");
    assert!(!repeat.status.success());
}

#[test]
fn rejects_an_invalid_component_name() {
    let directory = Directory::new("component-new-invalid");
    let output = kairo()
        .current_dir(&directory.0)
        .args(["component", "new", "Not Valid!"])
        .output()
        .expect("kairo component new should run");
    assert!(!output.status.success());
    assert!(!directory.0.join("components").exists());
}

/// exercises the real `cargo build` -> `wasm-tools component new` -> `wasm-tools validate`
/// pipeline end to end -- no mocking of the toolchain.
#[test]
fn builds_a_scaffolded_component_into_a_valid_wasm_component() {
    let directory = Directory::new("component-build");
    let scaffold = kairo()
        .current_dir(&directory.0)
        .args(["component", "new", "echo"])
        .output()
        .expect("kairo component new should run");
    assert!(scaffold.status.success(), "{scaffold:?}");

    let build = kairo()
        .current_dir(&directory.0)
        .args(["component", "build", "components/echo"])
        .output()
        .expect("kairo component build should run");
    assert!(build.status.success(), "{build:?}");

    let component_path = directory.0.join("components/echo/component.wasm");
    assert!(component_path.is_file());

    let check = kairo()
        .current_dir(&directory.0)
        .args(["component", "check"])
        .arg(&component_path)
        .output()
        .expect("kairo component check should run");
    assert!(check.status.success(), "{check:?}");
}
