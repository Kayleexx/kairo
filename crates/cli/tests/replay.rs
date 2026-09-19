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
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "kairo-replay-{}-{}",
            process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).expect("fixture directory should be created");
        Self(path)
    }

    fn run(&self, args: &[&str]) -> std::process::Output {
        kairo()
            .current_dir(&self.0)
            .args(args)
            .output()
            .expect("kairo command should run")
    }
}

impl Drop for Directory {
    fn drop(&mut self) {
        let _ = self.run(&["down"]);
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn replay_creates_an_immutable_child_from_a_durable_boundary() {
    let directory = Directory::new();
    let input = directory.0.join("input.bin");
    fs::write(&input, vec![7_u8; 512 * 1024]).expect("input");
    let transform = directory.0.join("transform.wat");
    fs::copy(repository_path("demos/stream/transform.wat"), &transform).expect("transform");
    let consume = repository_path("demos/stream/consume.wat");
    fs::write(
        directory.0.join("workflow.yaml"),
        format!(
            "workflow: replay-live\nmode: stream\ninput: {}\nsteps:\n  - name: checkpoint\n    component: {}\n  - name: relay\n    component: {}\n  - name: count\n    component: {}\nedges:\n  - from: checkpoint\n    to: relay\n    durability: required\n  - from: relay\n    to: count\n",
            input.display(),
            transform.display(),
            transform.display(),
            consume.display(),
        ),
    )
    .expect("workflow");
    let source = directory.run(&[
        "run",
        "workflow.yaml",
        "--watch",
        "--workers",
        "2",
        "--run",
        "replay-source",
    ]);
    assert!(source.status.success(), "{source:?}");
    let before = directory.run(&["inspect", "replay-source"]);
    assert!(before.status.success(), "{before:?}");

    let replay = directory.run(&["replay", "replay-source", "--until", "count"]);
    assert!(replay.status.success(), "{replay:?}");
    let replayed = String::from_utf8_lossy(&replay.stdout);
    let child = replayed
        .lines()
        .find_map(|line| line.strip_prefix("replay · "))
        .and_then(|line| line.split(" · ").next())
        .expect("child id should be printed");
    assert_ne!(child, "replay-source");

    let after = directory.run(&["inspect", "replay-source"]);
    assert_eq!(
        after.stdout, before.stdout,
        "source run must stay immutable"
    );
    let child_inspect = directory.run(&["inspect", child]);
    assert!(child_inspect.status.success(), "{child_inspect:?}");
    let rendered = String::from_utf8_lossy(&child_inspect.stdout);
    assert!(rendered.contains("replay · child of replay-source · through count"));
    assert!(rendered.contains("observed transport · remote-live"));
    let child_explain = directory.run(&["explain", child]);
    assert!(child_explain.status.success(), "{child_explain:?}");
    let explained = String::from_utf8_lossy(&child_explain.stdout);
    assert!(explained.contains("remote-live"));
    assert!(explained.contains("replay source       replay-source"));

    let state =
        fs::read_to_string(directory.0.join(".kairo/control-state.json")).expect("control state");
    assert!(
        state.contains(&format!("\"id\":\"{child}\"")) && state.contains("\"from_index\":1"),
        "child should resume from the committed durable boundary: {state}"
    );
}

#[test]
fn replay_rejects_an_invalid_target_without_creating_a_child() {
    let directory = Directory::new();
    let input = directory.0.join("input.bin");
    fs::write(&input, b"replay input").expect("input");
    let transform = repository_path("demos/stream/transform.wat");
    let consume = repository_path("demos/stream/consume.wat");
    fs::write(
        directory.0.join("workflow.yaml"),
        format!(
            "workflow: replay-invalid\nmode: stream\ninput: {}\nsteps:\n  - name: copy\n    component: {}\n  - name: count\n    component: {}\nedges:\n  - from: copy\n    to: count\n    durability: required\n",
            input.display(),
            transform.display(),
            consume.display(),
        ),
    )
    .expect("workflow");
    let source = directory.run(&[
        "run",
        "workflow.yaml",
        "--watch",
        "--workers",
        "2",
        "--run",
        "invalid-source",
    ]);
    assert!(source.status.success(), "{source:?}");
    let replay = directory.run(&["replay", "invalid-source", "--until", "missing"]);
    assert!(!replay.status.success());
    assert!(String::from_utf8_lossy(&replay.stderr).contains("not in the source workflow"));
    assert_eq!(
        fs::read_dir(directory.0.join(".kairo"))
            .expect("state directory")
            .filter_map(Result::ok)
            .filter(|entry| entry
                .file_name()
                .to_string_lossy()
                .contains("replay-invalid-replay"))
            .count(),
        0
    );
}

#[test]
fn replay_rejects_a_changed_component_before_scheduling_a_child() {
    let directory = Directory::new();
    let input = directory.0.join("input.bin");
    fs::write(&input, b"component lineage").expect("input");
    let transform = directory.0.join("transform.wat");
    fs::copy(repository_path("demos/stream/transform.wat"), &transform).expect("transform");
    let consume = repository_path("demos/stream/consume.wat");
    fs::write(
        directory.0.join("workflow.yaml"),
        format!(
            "workflow: replay-hash\nmode: stream\ninput: {}\nsteps:\n  - name: copy\n    component: {}\n  - name: count\n    component: {}\nedges:\n  - from: copy\n    to: count\n    durability: required\n",
            input.display(),
            transform.display(),
            consume.display(),
        ),
    )
    .expect("workflow");
    let source = directory.run(&[
        "run",
        "workflow.yaml",
        "--watch",
        "--workers",
        "2",
        "--run",
        "hash-source",
    ]);
    assert!(source.status.success(), "{source:?}");
    fs::write(&transform, "changed component").expect("changed component");
    let replay = directory.run(&["replay", "hash-source", "--until", "count"]);
    assert!(!replay.status.success());
    assert!(
        String::from_utf8_lossy(&replay.stderr).contains("component hash no longer matches"),
        "{replay:?}"
    );
}

#[test]
fn replay_without_a_boundary_reuses_the_verified_original_input() {
    let directory = Directory::new();
    let input = directory.0.join("input.bin");
    fs::write(&input, b"original input remains available").expect("input");
    let transform = repository_path("demos/stream/transform.wat");
    let consume = repository_path("demos/stream/consume.wat");
    fs::write(
        directory.0.join("workflow.yaml"),
        format!(
            "workflow: replay-input\nmode: stream\ninput: {}\nsteps:\n  - name: copy\n    component: {}\n  - name: count\n    component: {}\nedges:\n  - from: copy\n    to: count\n",
            input.display(),
            transform.display(),
            consume.display(),
        ),
    )
    .expect("workflow");
    let source = directory.run(&[
        "run",
        "workflow.yaml",
        "--watch",
        "--workers",
        "2",
        "--run",
        "input-source",
    ]);
    assert!(source.status.success(), "{source:?}");
    let replay = directory.run(&["replay", "input-source", "--until", "count"]);
    assert!(replay.status.success(), "{replay:?}");
    assert!(String::from_utf8_lossy(&replay.stdout).contains("source input-source"));
}

#[test]
fn replay_rejects_changed_original_input_when_no_boundary_exists() {
    let directory = Directory::new();
    let input = directory.0.join("input.bin");
    fs::write(&input, b"original input").expect("input");
    let transform = repository_path("demos/stream/transform.wat");
    let consume = repository_path("demos/stream/consume.wat");
    fs::write(
        directory.0.join("workflow.yaml"),
        format!(
            "workflow: replay-input-hash\nmode: stream\ninput: {}\nsteps:\n  - name: copy\n    component: {}\n  - name: count\n    component: {}\nedges:\n  - from: copy\n    to: count\n",
            input.display(),
            transform.display(),
            consume.display(),
        ),
    )
    .expect("workflow");
    let source = directory.run(&[
        "run",
        "workflow.yaml",
        "--watch",
        "--workers",
        "2",
        "--run",
        "input-hash-source",
    ]);
    assert!(source.status.success(), "{source:?}");
    fs::write(&input, b"changed input").expect("changed input");
    let replay = directory.run(&["replay", "input-hash-source", "--until", "count"]);
    assert!(!replay.status.success());
    assert!(String::from_utf8_lossy(&replay.stderr).contains("input hash no longer matches"));
}

#[test]
fn replay_uses_the_previous_valid_boundary_when_the_latest_artifact_is_missing() {
    let directory = Directory::new();
    let input = directory.0.join("input.bin");
    fs::write(&input, vec![9_u8; 128 * 1024]).expect("input");
    let transform = repository_path("demos/stream/transform.wat");
    let consume = repository_path("demos/stream/consume.wat");
    fs::write(
        directory.0.join("workflow.yaml"),
        format!(
            "workflow: replay-fallback\nmode: stream\ninput: {}\nsteps:\n  - name: one\n    component: {}\n  - name: two\n    component: {}\n  - name: relay\n    component: {}\n  - name: count\n    component: {}\nedges:\n  - from: one\n    to: two\n    durability: required\n  - from: two\n    to: relay\n    durability: required\n  - from: relay\n    to: count\n",
            input.display(),
            transform.display(),
            transform.display(),
            transform.display(),
            consume.display(),
        ),
    )
    .expect("workflow");
    let source = directory.run(&[
        "run",
        "workflow.yaml",
        "--watch",
        "--workers",
        "2",
        "--run",
        "fallback-source",
    ]);
    assert!(source.status.success(), "{source:?}");
    let state: serde_json::Value = serde_json::from_slice(
        &fs::read(directory.0.join(".kairo/control-state.json")).expect("control state"),
    )
    .expect("state json");
    let boundaries = state["runs"]["fallback-source"]["lineage"]["boundaries"]
        .as_array()
        .expect("boundaries");
    let latest = boundaries
        .last()
        .and_then(|boundary| boundary["artifact_hash"].as_str())
        .expect("latest boundary");
    fs::remove_file(directory.0.join(".kairo/artifacts/outputs").join(latest))
        .expect("remove latest artifact");

    let replay = directory.run(&["replay", "fallback-source", "--until", "count"]);
    assert!(replay.status.success(), "{replay:?}");
    // the selected latest boundary is gone. a successful child therefore proves it started from
    // the earlier valid boundary; its final request naturally records its own later yield.
}

#[test]
fn replay_rejects_a_changed_workflow_before_scheduling_a_child() {
    let directory = Directory::new();
    let input = directory.0.join("input.bin");
    fs::write(&input, b"workflow lineage").expect("input");
    let transform = repository_path("demos/stream/transform.wat");
    let consume = repository_path("demos/stream/consume.wat");
    let workflow = directory.0.join("workflow.yaml");
    fs::write(
        &workflow,
        format!(
            "workflow: replay-workflow-hash\nmode: stream\ninput: {}\nsteps:\n  - name: copy\n    component: {}\n  - name: count\n    component: {}\nedges:\n  - from: copy\n    to: count\n    durability: required\n",
            input.display(),
            transform.display(),
            consume.display(),
        ),
    )
    .expect("workflow");
    let source = directory.run(&[
        "run",
        "workflow.yaml",
        "--watch",
        "--workers",
        "2",
        "--run",
        "workflow-hash-source",
    ]);
    assert!(source.status.success(), "{source:?}");
    fs::write(
        &workflow,
        format!(
            "description: changed\n{}",
            fs::read_to_string(&workflow).expect("workflow source")
        ),
    )
    .expect("changed workflow");
    let replay = directory.run(&["replay", "workflow-hash-source", "--until", "count"]);
    assert!(!replay.status.success());
    assert!(String::from_utf8_lossy(&replay.stderr).contains("workflow hash no longer matches"));
}
