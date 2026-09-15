#![allow(clippy::expect_used)]

use std::{
    fs,
    path::PathBuf,
    process::{self, Command},
    sync::atomic::{AtomicU64, Ordering},
};

fn kairo() -> Command {
    Command::new(env!("CARGO_BIN_EXE_kairo"))
}

fn repository_path(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(path)
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

    /// copies the repo's own already-built `value-echo` component to `components/<name>/component.wasm`
    /// -- the exact layout `kairo component build` always produces, real and valid, no fixture forging.
    fn component(&self, name: &str) -> PathBuf {
        let directory = self.0.join("components").join(name);
        fs::create_dir_all(&directory).expect("component directory should be created");
        let destination = directory.join("component.wasm");
        fs::copy(
            repository_path("components/runtime/value-echo/component.wasm"),
            &destination,
        )
        .expect("component should copy");
        destination
    }
}

impl Drop for Directory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn step_names(source: &str) -> Vec<&str> {
    source
        .lines()
        .filter_map(|line| line.trim_start().strip_prefix("- name: \""))
        .filter_map(|line| line.strip_suffix('"'))
        .collect()
}

#[test]
fn two_different_component_wasm_outputs_get_distinct_directory_based_names() {
    let directory = Directory::new("step-names-dupe-wasm");
    let price = directory.component("price");
    let ship = directory.component("ship");

    let created = kairo()
        .args(["workflow", "create", "--name", "checkout", "--component"])
        .arg(&price)
        .args(["--component"])
        .arg(&ship)
        .current_dir(&directory.0)
        .output()
        .expect("workflow should be created");
    assert!(created.status.success(), "{created:?}");

    let source = fs::read_to_string(directory.0.join("checkout.yaml")).expect("workflow exists");
    assert_eq!(step_names(&source), ["price", "ship"]);
}

#[test]
fn meaningful_wat_filenames_are_preserved() {
    let directory = Directory::new("step-names-wat");
    let multiply = repository_path("demos/basic/multiply-by-nine.wat");
    let divide = repository_path("demos/basic/divide-by-five.wat");

    let created = kairo()
        .args(["workflow", "create", "--name", "math", "--component"])
        .arg(&multiply)
        .args(["--component"])
        .arg(&divide)
        .current_dir(&directory.0)
        .output()
        .expect("workflow should be created");
    assert!(created.status.success(), "{created:?}");

    let source = fs::read_to_string(directory.0.join("math.yaml")).expect("workflow exists");
    assert_eq!(step_names(&source), ["multiply-by-nine", "divide-by-five"]);
}

#[test]
fn remaining_directory_name_collisions_are_resolved_predictably() {
    let directory = Directory::new("step-names-dir-collision");
    // two different components.wasm whose immediate project directory is *also* named the same
    // ("foo" at two different paths) -- the directory-name fallback alone isn't enough here, so
    // this exercises the final numbered-suffix resolution.
    let first_dir = directory.0.join("components/foo");
    let second_dir = directory.0.join("nested/foo");
    fs::create_dir_all(&first_dir).expect("first directory should be created");
    fs::create_dir_all(&second_dir).expect("second directory should be created");
    let source_component = repository_path("components/runtime/value-echo/component.wasm");
    let first = first_dir.join("component.wasm");
    let second = second_dir.join("component.wasm");
    fs::copy(&source_component, &first).expect("first component should copy");
    fs::copy(&source_component, &second).expect("second component should copy");

    let created = kairo()
        .args(["workflow", "create", "--name", "collide", "--component"])
        .arg(&first)
        .args(["--component"])
        .arg(&second)
        .current_dir(&directory.0)
        .output()
        .expect("workflow should be created");
    assert!(created.status.success(), "{created:?}");

    let source = fs::read_to_string(directory.0.join("collide.yaml")).expect("workflow exists");
    let names = step_names(&source);
    assert_eq!(names, ["foo", "foo-2"]);
}

#[test]
fn generated_step_names_are_deterministic_across_runs() {
    let first_run = Directory::new("step-names-deterministic-a");
    let second_run = Directory::new("step-names-deterministic-b");
    for directory in [&first_run, &second_run] {
        let price = directory.component("price");
        let ship = directory.component("ship");
        let created = kairo()
            .args(["workflow", "create", "--name", "checkout", "--component"])
            .arg(&price)
            .args(["--component"])
            .arg(&ship)
            .current_dir(&directory.0)
            .output()
            .expect("workflow should be created");
        assert!(created.status.success(), "{created:?}");
    }

    let first = fs::read_to_string(first_run.0.join("checkout.yaml")).expect("workflow exists");
    let second = fs::read_to_string(second_run.0.join("checkout.yaml")).expect("workflow exists");
    assert_eq!(step_names(&first), step_names(&second));
    assert_eq!(step_names(&first), ["price", "ship"]);
}

// NOTE: explicit, user-typed step names (the interactive per-component prompt in `new.rs`'s
// `interactive()`) are structurally untouched by the batch-mode default-naming fix above -- that
// loop only ever runs when `--component` is omitted, and default-naming only ever runs in the
// opposite case, so the two never compete. Not covered by an automated test here: exercising the
// interactive prompt needs a real pty (`io::stdin().is_terminal()` gates it), and this repo has
// no pty test harness; adding one just for this would be disproportionate to the fix.
