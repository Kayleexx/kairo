#![allow(clippy::expect_used)]

use std::{
    fs,
    path::PathBuf,
    process,
    sync::atomic::{AtomicU64, Ordering},
};

use kairo_core::{Config, Workflow};
use kairo_runtime::{Runtime, inspect_cell};

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

fn workflow(steps: &[(&str, &str)]) -> Workflow {
    let step_yaml = steps
        .iter()
        .map(|(name, component)| {
            format!(
                "  - name: {name}\n    component: {}\n",
                repository_path(component).display()
            )
        })
        .collect::<String>();
    let edge_yaml = steps
        .windows(2)
        .map(|pair| format!("  - from: {}\n    to: {}\n", pair[0].0, pair[1].0))
        .collect::<String>();
    Workflow::parse(
        &format!(
            "workflow: recovery-timing-test\ninput: 21\nsteps:\n{step_yaml}edges:\n{edge_yaml}"
        ),
        std::path::Path::new("."),
        16,
    )
    .expect("workflow should parse")
}

#[tokio::test]
async fn a_fresh_cell_records_no_recovery_time() {
    let state = StateFile::new("fresh-cell");
    let workflow = workflow(&[("multiply", "demos/basic/multiply-by-nine.wat")]);
    let runtime = Runtime::new(Config::default()).expect("runtime should initialize");

    runtime
        .run_cell(&workflow, state.path(), None)
        .await
        .expect("cell should run");

    let inspection = inspect_cell(state.path()).expect("cell should be inspectable");
    assert_eq!(
        inspection.recovery_duration_us, None,
        "a brand new journal never recovered anything, so this must stay unknown, not a fake ~0"
    );
}

#[tokio::test]
async fn resuming_an_existing_journal_records_a_real_recovery_duration() {
    let state = StateFile::new("resumed-cell");
    let workflow = workflow(&[("multiply", "demos/basic/multiply-by-nine.wat")]);
    let runtime = Runtime::new(Config::default()).expect("runtime should initialize");

    runtime
        .run_cell(&workflow, state.path(), None)
        .await
        .expect("first run should complete");
    // a second open of the same journal is a real resume: this Cell replays existing events to
    // reconstruct its state before confirming it's already done, which is exactly the recovery
    // cost being measured.
    runtime
        .run_cell(&workflow, state.path(), None)
        .await
        .expect("second run should reconstruct from the journal");

    let inspection = inspect_cell(state.path()).expect("cell should be inspectable");
    inspection
        .recovery_duration_us
        .expect("a resumed cell must record a real recovery duration, not leave it unknown");
}

#[tokio::test]
async fn recovering_an_interrupted_component_records_a_real_recovery_duration() {
    let state = StateFile::new("interrupted-cell");
    let workflow = workflow(&[
        ("multiply", "demos/basic/multiply-by-nine.wat"),
        ("runaway", "components/runtime/runaway.wat"),
    ]);
    let runtime = Runtime::new(Config::default()).expect("runtime should initialize");

    let _ = runtime.run_cell(&workflow, state.path(), None).await;
    // a second attempt genuinely resumes a journal that was left mid-component after the first
    // (fuel-exhausted) attempt -- a real crash-recovery shape, not just a completed-cell reopen.
    let _ = runtime.run_cell(&workflow, state.path(), None).await;

    let inspection = inspect_cell(state.path()).expect("cell should be inspectable");
    inspection
        .recovery_duration_us
        .expect("recovering an interrupted component must record a real recovery duration");
}
