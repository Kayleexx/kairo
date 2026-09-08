#![allow(clippy::expect_used)]

use std::{fs, path::PathBuf, process};

use kairo_core::{Config, Workflow};
use kairo_runtime::{CellStatus, Runtime, inspect_cell};
use kairo_storage::ArtifactStore;
use rusqlite::Connection;

struct StateFile(PathBuf);

impl StateFile {
    fn new(name: &str) -> Self {
        Self(std::env::temp_dir().join(format!("kairo-{name}-{}.db", process::id())))
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
async fn inspects_completed_components_and_checkpoints() {
    let state = StateFile::new("completed-inspection");
    let runtime = Runtime::new(Config::default()).expect("runtime should initialize");
    let workflow = runtime
        .load_workflow(repository_path("demos/checkout/workflow.yaml"))
        .expect("workflow should load");
    runtime
        .run_cell(&workflow, state.path(), Some(&ArtifactStore::memory()))
        .await
        .expect("workflow should complete");

    let inspected = inspect_cell(state.path()).expect("cell should be inspected");

    assert_eq!(inspected.name.as_deref(), Some("checkout-settlement"));
    assert_eq!(inspected.input, 2500);
    assert_eq!(inspected.status, CellStatus::Completed { output: 3207 });
    assert_eq!(inspected.components.len(), 4);
    assert!(
        inspected
            .components
            .iter()
            .all(|component| component.duration_us.is_some())
    );
    assert_eq!(inspected.components[2].durable_after, Some(true));
    assert!(inspected.components[2].checkpoint.is_some());
}

#[tokio::test]
async fn identifies_an_interrupted_component() {
    let state = StateFile::new("interrupted-inspection");
    let runtime = Runtime::new(Config::default()).expect("runtime should initialize");
    let workflow = Workflow::parse(
        &format!(
            "workflow: interrupted\ninput: 1\nsteps:\n  - name: runaway\n    component: {}\nedges: []\n",
            repository_path("components/runtime/runaway.wat").display()
        ),
        std::path::Path::new("."),
        4,
    )
    .expect("workflow should parse");
    runtime
        .run_cell(&workflow, state.path(), None)
        .await
        .expect_err("runaway component should fail");

    let inspected = inspect_cell(state.path()).expect("cell should be inspected");

    assert_eq!(
        inspected.status,
        CellStatus::Interrupted {
            step: "runaway".to_owned()
        }
    );
    assert_eq!(inspected.components[0].attempts, 1);
    assert_eq!(inspected.components[0].output, None);
}

#[test]
fn reads_older_journals_without_modifying_them() {
    let state = StateFile::new("legacy-inspection");
    let connection = Connection::open(state.path()).expect("journal should open");
    connection
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
            INSERT INTO events(kind, workflow_fingerprint, input_value)
                VALUES ('workflow_started', 'fingerprint', 2);
            INSERT INTO events(kind, step_index, component_name, component_hash, input_value)
                VALUES ('component_started', 0, 'double', 'sha256:test', 2);
            INSERT INTO events(kind, step_index, output_value)
                VALUES ('component_completed', 0, 4);
            INSERT INTO events(kind, output_value) VALUES ('workflow_completed', 4);
            PRAGMA user_version = 2;",
        )
        .expect("older journal should be written");
    drop(connection);

    let inspected = inspect_cell(state.path()).expect("older cell should be inspected");
    let version = Connection::open(state.path())
        .expect("journal should reopen")
        .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
        .expect("version should load");

    assert_eq!(inspected.name, None);
    assert_eq!(inspected.components[0].duration_us, None);
    assert_eq!(inspected.status, CellStatus::Completed { output: 4 });
    assert_eq!(version, 2);
}

#[test]
fn identifies_a_checkpoint_that_was_not_written() {
    let state = StateFile::new("pending-checkpoint-inspection");
    write_current_events(
        state.path(),
        "
            INSERT INTO events(
                kind, workflow_fingerprint, input_value, workflow_name, component_count
            ) VALUES ('workflow_started', 'fingerprint', 2, 'pending-checkpoint', 1);
            INSERT INTO events(
                kind, step_index, component_name, component_hash, input_value,
                durability_required
            ) VALUES ('component_started', 0, 'double', 'sha256:test', 2, 1);
            INSERT INTO events(kind, step_index, output_value, duration_us)
                VALUES ('component_completed', 0, 4, 10);",
    );

    let inspected = inspect_cell(state.path()).expect("cell should be inspected");

    assert_eq!(
        inspected.status,
        CellStatus::CheckpointPending {
            step: "double".to_owned()
        }
    );
}

#[test]
fn identifies_a_cell_waiting_for_its_completion_event() {
    let state = StateFile::new("finalizing-inspection");
    write_current_events(
        state.path(),
        "
            INSERT INTO events(
                kind, workflow_fingerprint, input_value, workflow_name, component_count
            ) VALUES ('workflow_started', 'fingerprint', 2, 'finalizing', 1);
            INSERT INTO events(
                kind, step_index, component_name, component_hash, input_value,
                durability_required
            ) VALUES ('component_started', 0, 'double', 'sha256:test', 2, 0);
            INSERT INTO events(kind, step_index, output_value, duration_us)
                VALUES ('component_completed', 0, 4, 10);",
    );

    let inspected = inspect_cell(state.path()).expect("cell should be inspected");

    assert_eq!(inspected.status, CellStatus::Finalizing);
}

fn write_current_events(path: &std::path::Path, events: &str) {
    let connection = Connection::open(path).expect("journal should open");
    connection
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
                artifact_hash TEXT,
                workflow_name TEXT,
                duration_us INTEGER,
                durability_required INTEGER,
                component_count INTEGER
            ) STRICT;
            PRAGMA user_version = 3;",
        )
        .expect("journal schema should be written");
    connection
        .execute_batch(events)
        .expect("journal events should be written");
}
