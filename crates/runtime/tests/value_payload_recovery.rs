#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::{
    fs,
    path::PathBuf,
    process,
    sync::atomic::{AtomicU64, Ordering},
};

use kairo_core::{Config, Workflow};
use kairo_runtime::{LocalBlobError, Runtime, RuntimeError};
use kairo_storage::{ArtifactStore, StorageConfig};
use rusqlite::Connection;

const VALUE_ECHO: &str = "components/runtime/value-echo/component.wasm";
const VALUE_POISON: &str = "components/runtime/value-poison/component.wasm";
const LOCAL_BLOB_THRESHOLD: usize = 256 * 1024;

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
        let _ = fs::remove_dir_all(self.0.with_extension("blobs"));
        let _ = fs::remove_dir_all(self.0.with_extension("artifacts"));
    }
}

fn repository_path(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(path)
}

fn value_workflow(steps: &[(&str, &str)], durability: &[&str]) -> Workflow {
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
        .enumerate()
        .map(|(index, pair)| {
            let durability = durability.get(index).copied().unwrap_or("ephemeral");
            format!(
                "  - from: {}\n    to: {}\n    durability: {durability}\n",
                pair[0].0, pair[1].0
            )
        })
        .collect::<String>();
    Workflow::parse(
        &format!("workflow: value-test\nmode: value\nsteps:\n{step_yaml}edges:\n{edge_yaml}"),
        std::path::Path::new("."),
        16,
    )
    .expect("value workflow should parse")
}

fn incremented(input: &[u8], times: u8) -> Vec<u8> {
    input.iter().map(|byte| byte.wrapping_add(times)).collect()
}

fn local_artifact_store(state: &StateFile) -> ArtifactStore {
    let root = state.path().with_extension("artifacts");
    ArtifactStore::from_config(StorageConfig {
        endpoint: root.to_string_lossy().into_owned(),
        bucket: String::new(),
        local: true,
    })
    .expect("local artifact store should initialize")
}

#[tokio::test]
async fn inline_bytes_round_trip_and_resumes_without_rerunning() {
    let state = StateFile::new("value-inline");
    let workflow = value_workflow(&[("first", VALUE_ECHO), ("second", VALUE_ECHO)], &[]);
    let runtime = Runtime::new(Config::default()).expect("runtime should initialize");
    let input = b"hello, kairo".to_vec();
    let expected = incremented(&input, 2);

    let first = runtime
        .run_value_cell(&workflow, state.path(), None, input.clone())
        .await
        .expect("value cell should run");
    assert_eq!(first.output, expected);
    assert!(!first.resumed);

    let second = runtime
        .run_value_cell(&workflow, state.path(), None, input)
        .await
        .expect("value cell should reconstruct");
    assert_eq!(second.output, expected);
    assert!(second.resumed);
}

#[tokio::test]
async fn local_blob_round_trip_for_a_value_over_the_inline_threshold() {
    let state = StateFile::new("value-local-blob");
    let workflow = value_workflow(&[("first", VALUE_ECHO), ("second", VALUE_ECHO)], &[]);
    let runtime = Runtime::new(Config::default()).expect("runtime should initialize");
    let input = vec![7_u8; LOCAL_BLOB_THRESHOLD + 1024];
    let expected = incremented(&input, 2);

    let result = runtime
        .run_value_cell(&workflow, state.path(), None, input)
        .await
        .expect("value cell should run");
    assert_eq!(result.output, expected);

    // three distinct over-threshold values go through a local blob: the workflow's own input
    // (all 7s), step one's output (all 8s), and step two's/the final output (all 9s).
    let blobs_dir = state.path().with_extension("blobs");
    let entries: Vec<_> = fs::read_dir(&blobs_dir)
        .expect("local blob directory should exist")
        .collect::<Result<_, _>>()
        .expect("blob directory entries should read");
    assert_eq!(
        entries.len(),
        3,
        "expected one local blob per distinct over-threshold value, found {entries:?}"
    );
}

#[tokio::test]
async fn a_missing_local_blob_fails_recovery_loudly() {
    let state = StateFile::new("value-missing-blob");
    let workflow = value_workflow(&[("first", VALUE_ECHO)], &[]);
    let runtime = Runtime::new(Config::default()).expect("runtime should initialize");
    let input = vec![9_u8; LOCAL_BLOB_THRESHOLD + 1024];

    let first = runtime
        .run_value_cell(&workflow, state.path(), None, input.clone())
        .await
        .expect("value cell should run");
    assert_eq!(first.output.len(), input.len());

    let blobs_dir = state.path().with_extension("blobs");
    let mut removed_any = false;
    for entry in fs::read_dir(&blobs_dir).expect("local blob directory should exist") {
        fs::remove_file(entry.expect("entry should read").path())
            .expect("local blob file should delete");
        removed_any = true;
    }
    assert!(
        removed_any,
        "test setup should have produced at least one local blob to delete"
    );

    // this worker's disk lost the value's only copy -- a local payload is never recoverable
    // elsewhere, so this must fail loudly, never silently substitute or panic.
    let error = runtime
        .run_value_cell(&workflow, state.path(), None, input)
        .await
        .expect_err("recovery should fail when the local blob is gone");
    assert!(matches!(
        error,
        RuntimeError::LocalBlob {
            source: LocalBlobError::Missing { .. }
        }
    ));
}

fn component_started_count(path: &std::path::Path, index: i64) -> i64 {
    Connection::open(path)
        .expect("journal should open")
        .query_row(
            "SELECT COUNT(*) FROM events WHERE kind = 'component_started' AND step_index = ?1",
            [index],
            |row| row.get(0),
        )
        .expect("event count should load")
}

#[tokio::test]
async fn a_required_edge_checkpoint_is_never_rerun_and_rejects_a_missing_or_mismatched_backend() {
    let state = StateFile::new("value-durable-checkpoint");
    let workflow = value_workflow(
        &[("first", VALUE_ECHO), ("second", VALUE_POISON)],
        &["required"],
    );
    let runtime = Runtime::new(Config::default()).expect("runtime should initialize");
    let artifacts = ArtifactStore::memory();
    let input = b"durable checkpoint".to_vec();

    // "first" checkpoints durably (required edge); "second" always fails, so the workflow never
    // completes -- exactly what exercises repeated recovery into the same checkpoint.
    let first = runtime
        .run_value_cell(&workflow, state.path(), Some(&artifacts), input.clone())
        .await
        .expect_err("the poisoned second step should fail");
    assert!(matches!(first, RuntimeError::WorkflowStep { .. }));

    let missing = ArtifactStore::memory();
    let error = runtime
        .run_value_cell(&workflow, state.path(), Some(&missing), input.clone())
        .await
        .expect_err("recovery should require the durably stored checkpoint");
    assert!(
        matches!(error, RuntimeError::Artifact { .. }),
        "expected a missing-artifact error, got {error:?}"
    );

    let local = local_artifact_store(&state);
    let error = runtime
        .run_value_cell(&workflow, state.path(), Some(&local), input.clone())
        .await
        .expect_err("recovery through a different backend should fail");
    assert!(matches!(
        error,
        RuntimeError::CheckpointBackendMismatch {
            recorded,
            configured: "local",
            ..
        } if recorded == "memory"
    ));

    let second = runtime
        .run_value_cell(&workflow, state.path(), Some(&artifacts), input)
        .await
        .expect_err("the poisoned step fails deterministically on every attempt");
    assert!(matches!(second, RuntimeError::WorkflowStep { .. }));

    // across all three attempts, the durably checkpointed first step must never re-execute.
    assert_eq!(component_started_count(state.path(), 0), 1);
}

fn payload_kind(path: &std::path::Path, sql: &str, params: &[&dyn rusqlite::ToSql]) -> String {
    Connection::open(path)
        .expect("journal should open")
        .query_row(sql, params, |row| row.get(0))
        .expect("payload kind should load")
}

#[tokio::test]
async fn a_step_s_input_and_the_workflow_s_output_are_journaled_as_reuse_not_duplicated() {
    let state = StateFile::new("value-reuse");
    let workflow = value_workflow(&[("first", VALUE_ECHO), ("second", VALUE_ECHO)], &[]);
    let runtime = Runtime::new(Config::default()).expect("runtime should initialize");
    let input = vec![5_u8; LOCAL_BLOB_THRESHOLD + 1024];

    runtime
        .run_value_cell(&workflow, state.path(), None, input)
        .await
        .expect("value cell should run");

    // "second"'s input is identical to "first"'s just-produced output -- never worth a second
    // local blob write of the same bytes.
    let second_input_kind = payload_kind(
        state.path(),
        "SELECT input_payload_kind FROM events WHERE kind = 'component_started' AND step_index = 1",
        &[],
    );
    assert_eq!(second_input_kind, "reuse");

    // the workflow's final output is identical to "second"'s output -- same rule at completion.
    let completed_output_kind = payload_kind(
        state.path(),
        "SELECT output_payload_kind FROM events WHERE kind = 'workflow_completed'",
        &[],
    );
    assert_eq!(completed_output_kind, "reuse");

    // three distinct values exist (workflow input, "first" output, "second" output) despite five
    // payload-carrying events -- "second"'s input and the workflow's completed output are each a
    // reuse of an already-written value, so they must not add a fourth or fifth blob.
    let blobs_dir = state.path().with_extension("blobs");
    let entries: Vec<_> = fs::read_dir(&blobs_dir)
        .expect("local blob directory should exist")
        .collect::<Result<_, _>>()
        .expect("blob directory entries should read");
    assert_eq!(
        entries.len(),
        3,
        "reused payloads must not duplicate the underlying blob, found {entries:?}"
    );
}
