#![allow(clippy::expect_used)]

use std::{
    fs,
    path::PathBuf,
    process,
    sync::atomic::{AtomicU64, Ordering},
};

use kairo_core::{Config, Workflow};
use kairo_runtime::{Runtime, inspect_cell};
use kairo_storage::ArtifactStore;

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

struct StateFile(PathBuf);

impl StateFile {
    fn new(name: &str) -> Self {
        let sequence = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        Self(std::env::temp_dir().join(format!("kairo-{name}-{}-{sequence}.db", process::id())))
    }

    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for StateFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
        let _ = fs::remove_file(self.0.with_extension("db-journal"));
    }
}

fn repository_path(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(path)
}

#[tokio::test]
async fn records_real_checkpoint_bytes_and_duration() {
    let state = StateFile::new("checkpoint-metrics");
    let workflow = Workflow::parse(
        &format!(
            "workflow: checkpoint-metrics-test\ninput: 21\nsteps:\n  - name: multiply\n    component: {}\n  - name: divide\n    component: {}\nedges:\n  - from: multiply\n    to: divide\n    durability: required\n",
            repository_path("demos/basic/multiply-by-nine.wat").display(),
            repository_path("demos/basic/divide-by-five.wat").display(),
        ),
        std::path::Path::new("."),
        16,
    )
    .expect("workflow should parse");
    let runtime = Runtime::new(Config::default()).expect("runtime should initialize");
    let artifacts = ArtifactStore::memory();

    runtime
        .run_cell(&workflow, state.path(), Some(&artifacts))
        .await
        .expect("workflow should complete");

    let inspection = inspect_cell(state.path()).expect("cell should be inspectable");
    let checkpointed = inspection
        .components
        .iter()
        .find(|component| component.checkpoint.is_some())
        .expect("one component should have a checkpoint");

    let bytes = checkpointed
        .checkpoint_bytes
        .expect("checkpoint bytes should be recorded, not left unknown");
    assert!(bytes > 0, "a stored checkpoint is never zero bytes");
    checkpointed
        .checkpoint_duration_us
        .expect("checkpoint duration should be recorded, not left unknown");
}

#[tokio::test]
async fn leaves_checkpoint_metrics_unset_for_ephemeral_edges() {
    let state = StateFile::new("checkpoint-metrics-ephemeral");
    let workflow = Workflow::parse(
        &format!(
            "workflow: no-checkpoint-test\ninput: 21\nsteps:\n  - name: multiply\n    component: {}\n  - name: divide\n    component: {}\nedges:\n  - from: multiply\n    to: divide\n",
            repository_path("demos/basic/multiply-by-nine.wat").display(),
            repository_path("demos/basic/divide-by-five.wat").display(),
        ),
        std::path::Path::new("."),
        16,
    )
    .expect("workflow should parse");
    let runtime = Runtime::new(Config::default()).expect("runtime should initialize");

    runtime
        .run_cell(&workflow, state.path(), None)
        .await
        .expect("workflow should complete");

    let inspection = inspect_cell(state.path()).expect("cell should be inspectable");
    assert!(
        inspection
            .components
            .iter()
            .all(|component| component.checkpoint_bytes.is_none()
                && component.checkpoint_duration_us.is_none()),
        "an ephemeral edge never wrote a checkpoint, so its metrics must stay unknown, not zero"
    );
}
