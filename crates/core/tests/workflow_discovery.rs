#![allow(clippy::expect_used)]

use std::{
    fs,
    path::PathBuf,
    process,
    sync::atomic::{AtomicU64, Ordering},
};

use kairo_core::{Config, discover_shallow};

fn temp_dir(name: &str) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "kairo-discovery-{name}-{}-{sequence}",
        process::id()
    ));
    fs::create_dir_all(&path).expect("fixture directory should be created");
    path
}

// `kairo new`/`kairo workflow create` write directly into the project root -- this is the
// discovery path the TUI's Launch screen and `kairo workflows` both rely on to find them.
#[test]
fn finds_a_workflow_written_directly_to_the_scanned_directory() {
    let root = temp_dir("root-workflow");
    fs::write(
        root.join("numbers.yaml"),
        "workflow: numbers\nmode: value\nio:\n  input: value\n  output: value\nsteps:\n- name: only\n  component: ./component.wasm\nedges: []\n",
    )
    .expect("workflow should write");

    let found = discover_shallow(&root, Config::default());

    assert_eq!(found.len(), 1);
    assert_eq!(found[0].workflow.name(), "numbers");
    assert_eq!(found[0].path, root.join("numbers.yaml"));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn does_not_recurse_into_subdirectories() {
    let root = temp_dir("no-recurse");
    let nested = root.join("workflows");
    fs::create_dir_all(&nested).expect("nested directory should be created");
    fs::write(
        nested.join("nested.yaml"),
        "workflow: nested\nmode: value\nio:\n  input: value\n  output: value\nsteps:\n- name: only\n  component: ./component.wasm\nedges: []\n",
    )
    .expect("nested workflow should write");

    let found = discover_shallow(&root, Config::default());

    assert!(found.is_empty(), "should not recurse into subdirectories");

    let _ = fs::remove_dir_all(root);
}

#[test]
fn skips_an_unparseable_yaml_file_instead_of_erroring() {
    let root = temp_dir("unparseable");
    fs::write(
        root.join("not-a-workflow.yaml"),
        "this: is not a workflow\n",
    )
    .expect("file should write");

    let found = discover_shallow(&root, Config::default());

    assert!(found.is_empty());

    let _ = fs::remove_dir_all(root);
}
