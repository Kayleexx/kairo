#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::{
    collections::HashMap,
    fs,
    path::PathBuf,
    process,
    sync::atomic::{AtomicU64, Ordering},
};

use kairo_core::{Config, Workflow};
use kairo_runtime::{DurabilityProfile, Runtime, RuntimeError, WorkflowProfile, inspect_cell};
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

struct Profile(PathBuf);

impl Profile {
    fn write(shape: &str, edge: &str, profile: DurabilityProfile) -> Self {
        let path = PathBuf::from(".kairo/profiles").join(format!("{shape}.json"));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut edges = HashMap::new();
        edges.insert(edge.to_owned(), profile);
        let content = WorkflowProfile {
            workflow: "durability-auto-test".to_owned(),
            shape: shape.to_owned(),
            edges,
        };
        fs::write(&path, serde_json::to_vec(&content).unwrap()).unwrap();
        Self(path)
    }
}

impl Drop for Profile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn repository_path(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(path)
}

fn auto_workflow(name: &str) -> Workflow {
    Workflow::parse(
        &format!(
            "workflow: {name}\ninput: 21\nsteps:\n  - name: multiply\n    component: {}\n  - name: divide\n    component: {}\nedges:\n  - from: multiply\n    to: divide\n    durability: auto\n",
            repository_path("demos/basic/multiply-by-nine.wat").display(),
            repository_path("demos/basic/divide-by-five.wat").display(),
        ),
        std::path::Path::new("."),
        16,
    )
    .expect("workflow should parse")
}

#[tokio::test]
async fn fails_clearly_when_no_profile_exists() {
    let state = StateFile::new("auto-no-profile");
    let workflow = auto_workflow("auto-no-profile-test");
    let runtime = Runtime::new(Config::default()).expect("runtime should initialize");
    let artifacts = ArtifactStore::memory();

    let error = runtime
        .run_cell(&workflow, state.path(), Some(&artifacts))
        .await
        .expect_err("auto edge with no profile must fail, never guess");

    assert!(matches!(
        error,
        RuntimeError::DurabilityProfileMissing { step } if step == "multiply"
    ));
}

#[tokio::test]
async fn resolves_ephemeral_when_recompute_is_cheap() {
    let state = StateFile::new("auto-ephemeral");
    let workflow = auto_workflow("auto-ephemeral-test");
    let runtime = Runtime::new(Config::default()).expect("runtime should initialize");
    let shape = runtime.workflow_shape(&workflow).expect("shape");
    let _profile = Profile::write(
        &shape,
        "multiply",
        DurabilityProfile {
            recompute_us: 10,
            checkpoint_bytes: 19,
            checkpoint_us: 5000,
            samples: 3,
            ..DurabilityProfile::empty(0)
        },
    );
    let artifacts = ArtifactStore::memory();

    runtime
        .run_cell(&workflow, state.path(), Some(&artifacts))
        .await
        .expect("workflow should complete");

    let inspection = inspect_cell(state.path()).expect("cell should be inspectable");
    let multiply = &inspection.components[0];
    assert!(
        multiply.checkpoint.is_none(),
        "cheap recompute stays ephemeral"
    );
    assert!(
        multiply
            .durability_reason
            .as_ref()
            .is_some_and(|reason| reason.contains("<=")),
        "reason should record why it stayed ephemeral"
    );
}

#[tokio::test]
async fn resolves_required_when_recompute_is_expensive() {
    let state = StateFile::new("auto-required");
    let workflow = auto_workflow("auto-required-test");
    let runtime = Runtime::new(Config::default()).expect("runtime should initialize");
    let shape = runtime.workflow_shape(&workflow).expect("shape");
    let _profile = Profile::write(
        &shape,
        "multiply",
        DurabilityProfile {
            recompute_us: 5000,
            checkpoint_bytes: 19,
            checkpoint_us: 10,
            samples: 3,
            ..DurabilityProfile::empty(0)
        },
    );
    let artifacts = ArtifactStore::memory();

    runtime
        .run_cell(&workflow, state.path(), Some(&artifacts))
        .await
        .expect("workflow should complete");

    let inspection = inspect_cell(state.path()).expect("cell should be inspectable");
    let multiply = &inspection.components[0];
    assert!(
        multiply.checkpoint.is_some(),
        "expensive recompute should checkpoint"
    );
    assert!(multiply.checkpoint_bytes.is_some_and(|bytes| bytes > 0));
    assert!(
        multiply
            .durability_reason
            .as_ref()
            .is_some_and(|reason| reason.contains('>'))
    );
}

#[tokio::test]
async fn recovery_reuses_the_persisted_plan_not_a_changed_profile() {
    let state = StateFile::new("auto-persisted-plan");
    let workflow = auto_workflow("auto-persisted-plan-test");
    let runtime = Runtime::new(Config::default()).expect("runtime should initialize");
    let shape = runtime.workflow_shape(&workflow).expect("shape");
    let profile = Profile::write(
        &shape,
        "multiply",
        DurabilityProfile {
            recompute_us: 5000,
            checkpoint_bytes: 19,
            checkpoint_us: 10,
            samples: 3,
            ..DurabilityProfile::empty(0)
        },
    );
    let artifacts = ArtifactStore::memory();

    runtime
        .run_cell(&workflow, state.path(), Some(&artifacts))
        .await
        .expect("first run should resolve to required and complete");

    // delete the profile the decision was based on -- a resumed cell must not need it again.
    drop(profile);

    let resumed = runtime
        .run_cell(&workflow, state.path(), Some(&artifacts))
        .await
        .expect("reopening a completed cell must not recompute the plan");
    assert!(resumed.resumed);

    let inspection = inspect_cell(state.path()).expect("cell should be inspectable");
    let multiply = &inspection.components[0];
    assert!(
        multiply.checkpoint.is_some(),
        "the original required decision must still hold after the profile is gone"
    );
}

#[tokio::test]
async fn auto_never_weakens_an_explicit_required_edge() {
    let state = StateFile::new("explicit-required");
    let workflow = Workflow::parse(
        &format!(
            "workflow: explicit-required-test\ninput: 21\nsteps:\n  - name: multiply\n    component: {}\n  - name: divide\n    component: {}\nedges:\n  - from: multiply\n    to: divide\n    durability: required\n",
            repository_path("demos/basic/multiply-by-nine.wat").display(),
            repository_path("demos/basic/divide-by-five.wat").display(),
        ),
        std::path::Path::new("."),
        16,
    )
    .expect("workflow should parse");
    let runtime = Runtime::new(Config::default()).expect("runtime should initialize");
    let artifacts = ArtifactStore::memory();

    // no profile exists anywhere for this shape -- an explicit `required` edge must not care.
    runtime
        .run_cell(&workflow, state.path(), Some(&artifacts))
        .await
        .expect("explicit required edges bypass the planner entirely");

    let inspection = inspect_cell(state.path()).expect("cell should be inspectable");
    assert!(inspection.components[0].checkpoint.is_some());
}
