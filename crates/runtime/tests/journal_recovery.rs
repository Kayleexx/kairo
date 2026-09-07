#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::{
    fs,
    path::PathBuf,
    process,
    sync::atomic::{AtomicU64, Ordering},
};

use kairo_core::{Config, Workflow};
use kairo_runtime::{JournalError, Runtime, RuntimeError};
use rusqlite::Connection;

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
        &format!("workflow: journal-test\ninput: 21\nsteps:\n{step_yaml}edges:\n{edge_yaml}"),
        std::path::Path::new("."),
        16,
    )
    .expect("workflow should parse")
}

#[tokio::test]
async fn returns_a_completed_cell_without_rerunning_it() {
    let state = StateFile::new("completed-cell");
    let workflow = workflow(&[("multiply", "demos/basic/multiply-by-nine.wat")]);
    let runtime = Runtime::new(Config::default()).expect("runtime should initialize");

    let first = runtime
        .run_cell(&workflow, state.path())
        .await
        .expect("cell should run");
    let second = runtime
        .run_cell(&workflow, state.path())
        .await
        .expect("cell should reconstruct");

    assert_eq!(first.output, 189);
    assert!(!first.resumed);
    assert_eq!(second.output, 189);
    assert!(second.resumed);
    assert_eq!(event_count(state.path(), "component_started", 0), 1);
}

#[tokio::test]
async fn reruns_only_an_incomplete_component() {
    let state = StateFile::new("incomplete-component");
    let workflow = workflow(&[
        ("multiply", "demos/basic/multiply-by-nine.wat"),
        ("runaway", "components/runtime/runaway.wat"),
    ]);
    let runtime = Runtime::new(Config::default()).expect("runtime should initialize");

    for _ in 0..2 {
        let error = runtime
            .run_cell(&workflow, state.path())
            .await
            .expect_err("runaway component should fail");
        assert!(matches!(error, RuntimeError::WorkflowStep { .. }));
    }

    assert_eq!(event_count(state.path(), "component_started", 0), 1);
    assert_eq!(event_count(state.path(), "component_completed", 0), 1);
    assert_eq!(event_count(state.path(), "component_started", 1), 2);
    assert_eq!(event_count(state.path(), "component_completed", 1), 0);
}

#[tokio::test]
async fn rejects_a_changed_workflow() {
    let state = StateFile::new("changed-workflow");
    let first = workflow(&[("stage", "demos/basic/multiply-by-nine.wat")]);
    let changed = workflow(&[("stage", "demos/basic/divide-by-five.wat")]);
    let runtime = Runtime::new(Config::default()).expect("runtime should initialize");
    runtime
        .run_cell(&first, state.path())
        .await
        .expect("first workflow should run");

    let error = runtime
        .run_cell(&changed, state.path())
        .await
        .expect_err("changed workflow should fail");

    assert!(matches!(
        error,
        RuntimeError::Journal {
            source: JournalError::Corrupt { .. },
            ..
        }
    ));
}

#[tokio::test]
async fn rejects_corrupt_event_history() {
    let state = StateFile::new("corrupt-history");
    let workflow = workflow(&[("stage", "demos/basic/multiply-by-nine.wat")]);
    let runtime = Runtime::new(Config::default()).expect("runtime should initialize");
    runtime
        .run_cell(&workflow, state.path())
        .await
        .expect("workflow should run");
    Connection::open(state.path())
        .expect("journal should open")
        .execute("INSERT INTO events(kind) VALUES ('unknown')", [])
        .expect("invalid event should be inserted");

    let error = runtime
        .run_cell(&workflow, state.path())
        .await
        .expect_err("corrupt history should fail");

    assert!(matches!(
        error,
        RuntimeError::Journal {
            source: JournalError::Corrupt { .. },
            ..
        }
    ));
}

#[tokio::test]
async fn rejects_an_unsupported_schema() {
    let state = StateFile::new("unsupported-schema");
    Connection::open(state.path())
        .expect("journal should open")
        .pragma_update(None, "user_version", 2)
        .expect("schema version should be written");
    let workflow = workflow(&[("stage", "demos/basic/multiply-by-nine.wat")]);
    let runtime = Runtime::new(Config::default()).expect("runtime should initialize");

    let error = runtime
        .run_cell(&workflow, state.path())
        .await
        .expect_err("unsupported schema should fail");

    assert!(matches!(
        error,
        RuntimeError::Journal {
            source: JournalError::UnsupportedSchema { found: 2 },
            ..
        }
    ));
}

#[tokio::test]
async fn rejects_a_cell_that_is_already_open() {
    let state = StateFile::new("busy-cell");
    let workflow = workflow(&[("stage", "demos/basic/multiply-by-nine.wat")]);
    let runtime = Runtime::new(Config::default()).expect("runtime should initialize");
    runtime
        .run_cell(&workflow, state.path())
        .await
        .expect("workflow should run");
    let connection = Connection::open(state.path()).expect("journal should open");
    connection
        .pragma_update(None, "locking_mode", "EXCLUSIVE")
        .expect("exclusive mode should configure");
    connection
        .execute_batch("BEGIN EXCLUSIVE; COMMIT;")
        .expect("exclusive lock should be acquired");

    let error = runtime
        .run_cell(&workflow, state.path())
        .await
        .expect_err("busy cell should fail");

    assert!(matches!(
        error,
        RuntimeError::Journal {
            source: JournalError::Busy { .. },
            ..
        }
    ));
}

#[tokio::test]
async fn rejects_state_for_stream_workflows() {
    let state = StateFile::new("stream-cell");
    let runtime = Runtime::new(Config::default()).expect("runtime should initialize");
    let workflow = runtime
        .load_workflow(repository_path("demos/stream/workflow.yaml"))
        .expect("workflow should load");

    assert!(matches!(
        runtime.run_cell(&workflow, state.path()).await,
        Err(RuntimeError::StatefulStreamWorkflow)
    ));
    assert!(!state.path().exists());
}

fn event_count(path: &std::path::Path, kind: &str, index: i64) -> i64 {
    Connection::open(path)
        .expect("journal should open")
        .query_row(
            "SELECT COUNT(*) FROM events WHERE kind = ?1 AND step_index = ?2",
            (kind, index),
            |row| row.get(0),
        )
        .expect("event count should load")
}
