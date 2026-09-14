#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::{
    fs,
    path::PathBuf,
    process,
    sync::atomic::{AtomicU64, Ordering},
};

use kairo_core::{Config, Workflow};
use kairo_runtime::{JournalError, Runtime, RuntimeError};
use kairo_storage::{ArtifactStore, StorageConfig, StorageError};
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
        .run_cell(&workflow, state.path(), None)
        .await
        .expect("cell should run");
    let second = runtime
        .run_cell(&workflow, state.path(), None)
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
            .run_cell(&workflow, state.path(), None)
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
        .run_cell(&first, state.path(), None)
        .await
        .expect("first workflow should run");

    let error = runtime
        .run_cell(&changed, state.path(), None)
        .await
        .expect_err("changed workflow should fail");

    assert!(matches!(
        error,
        RuntimeError::Journal {
            source: JournalError::WorkflowChanged,
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
        .run_cell(&workflow, state.path(), None)
        .await
        .expect("workflow should run");
    Connection::open(state.path())
        .expect("journal should open")
        .execute("INSERT INTO events(kind) VALUES ('unknown')", [])
        .expect("invalid event should be inserted");

    let error = runtime
        .run_cell(&workflow, state.path(), None)
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
        .pragma_update(None, "user_version", 6)
        .expect("schema version should be written");
    let workflow = workflow(&[("stage", "demos/basic/multiply-by-nine.wat")]);
    let runtime = Runtime::new(Config::default()).expect("runtime should initialize");

    let error = runtime
        .run_cell(&workflow, state.path(), None)
        .await
        .expect_err("unsupported schema should fail");

    assert!(matches!(
        error,
        RuntimeError::Journal {
            source: JournalError::UnsupportedSchema { found: 6 },
            ..
        }
    ));
}

#[tokio::test]
async fn migrates_older_journals() {
    let state = StateFile::new("journal-migration");
    Connection::open(state.path())
        .expect("journal should open")
        .execute_batch(
            "CREATE TABLE events (
                sequence INTEGER PRIMARY KEY,
                kind TEXT NOT NULL,
                step_index INTEGER,
                workflow_fingerprint TEXT,
                component_name TEXT,
                component_hash TEXT,
                input_value INTEGER,
                output_value INTEGER
            ) STRICT;
            PRAGMA user_version = 1;",
        )
        .expect("older schema should be written");
    let workflow = workflow(&[("stage", "demos/basic/multiply-by-nine.wat")]);
    let runtime = Runtime::new(Config::default()).expect("runtime should initialize");

    runtime
        .run_cell(&workflow, state.path(), None)
        .await
        .expect("migrated journal should run");

    let version = Connection::open(state.path())
        .expect("journal should open")
        .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
        .expect("schema version should load");
    assert_eq!(version, 5);
}

#[tokio::test]
async fn migrates_checkpoint_journals() {
    let state = StateFile::new("checkpoint-journal-migration");
    Connection::open(state.path())
        .expect("journal should open")
        .execute_batch(
            "CREATE TABLE events (
                sequence INTEGER PRIMARY KEY,
                kind TEXT NOT NULL,
                step_index INTEGER,
                workflow_fingerprint TEXT,
                component_name TEXT,
                component_hash TEXT,
                input_value INTEGER,
                output_value INTEGER,
                artifact_hash TEXT
            ) STRICT;
            PRAGMA user_version = 2;",
        )
        .expect("checkpoint schema should be written");
    let workflow = workflow(&[("stage", "demos/basic/multiply-by-nine.wat")]);
    let runtime = Runtime::new(Config::default()).expect("runtime should initialize");

    runtime
        .run_cell(&workflow, state.path(), None)
        .await
        .expect("migrated journal should run");

    let version = Connection::open(state.path())
        .expect("journal should open")
        .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
        .expect("schema version should load");
    assert_eq!(version, 5);
}

#[tokio::test]
async fn rejects_a_cell_that_is_already_open() {
    let state = StateFile::new("busy-cell");
    let workflow = workflow(&[("stage", "demos/basic/multiply-by-nine.wat")]);
    let runtime = Runtime::new(Config::default()).expect("runtime should initialize");
    runtime
        .run_cell(&workflow, state.path(), None)
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
        .run_cell(&workflow, state.path(), None)
        .await
        .expect_err("busy cell should fail");

    assert!(matches!(
        error,
        RuntimeError::Journal {
            source: JournalError::Busy,
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
        runtime.run_cell(&workflow, state.path(), None).await,
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

#[tokio::test]
async fn restores_required_checkpoints_from_artifact_storage() {
    let state = StateFile::new("required-checkpoint");
    let workflow = Workflow::parse(
        &format!(
            "workflow: checkpoint-test\ninput: 21\nsteps:\n  - name: multiply\n    component: {}\n  - name: divide\n    component: {}\n  - name: runaway\n    component: {}\nedges:\n  - from: multiply\n    to: divide\n  - from: divide\n    to: runaway\n    durability: required\n",
            repository_path("demos/basic/multiply-by-nine.wat").display(),
            repository_path("demos/basic/divide-by-five.wat").display(),
            repository_path("components/runtime/runaway.wat").display(),
        ),
        std::path::Path::new("."),
        16,
    )
    .expect("workflow should parse");
    let runtime = Runtime::new(Config::default()).expect("runtime should initialize");
    let artifacts = ArtifactStore::memory();

    let first = runtime
        .run_cell(&workflow, state.path(), Some(&artifacts))
        .await
        .expect_err("runaway component should fail");
    assert!(matches!(first, RuntimeError::WorkflowStep { .. }));

    let missing = ArtifactStore::memory();
    let error = runtime
        .run_cell(&workflow, state.path(), Some(&missing))
        .await
        .expect_err("recovery should require the stored checkpoint");
    assert!(matches!(
        error,
        RuntimeError::Artifact {
            source: StorageError::Missing { .. }
        }
    ));

    let artifact_root = state.path().with_extension("artifacts");
    let local = ArtifactStore::from_config(StorageConfig {
        endpoint: artifact_root.to_string_lossy().into_owned(),
        bucket: String::new(),
        local: true,
    })
    .expect("local store should initialize");
    let error = runtime
        .run_cell(&workflow, state.path(), Some(&local))
        .await
        .expect_err("recovery through another backend should fail");
    assert!(matches!(
        error,
        RuntimeError::CheckpointBackendMismatch {
            recorded,
            configured: "local",
            ..
        } if recorded == "memory"
    ));
    fs::remove_dir_all(artifact_root).expect("artifact directory should be removed");
    assert_eq!(event_count(state.path(), "component_started", 0), 1);
    assert_eq!(event_count(state.path(), "component_started", 1), 1);
    assert_eq!(event_count(state.path(), "component_started", 2), 1);
}
