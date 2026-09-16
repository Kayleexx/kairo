#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    process,
    sync::atomic::{AtomicU64, Ordering},
};

use kairo_core::{Config, Workflow};
use kairo_runtime::{
    AutoResolution, DurabilityProfile, GroupOutcome, Runtime, RuntimeError, inspect_cell,
};
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

// two steps, multiply then divide, joined by a `durability: auto` edge -- deliberately never
// resolved by a real profile, since these tests prove `run_cell_group` accepts an
// already-decided plan and never re-derives it from `.kairo/profiles/`.
fn two_step_workflow(input: u32) -> Workflow {
    Workflow::parse(
        &format!(
            "workflow: execution-groups-test\ninput: {input}\nsteps:\n  - name: multiply\n    component: {}\n  - name: divide\n    component: {}\nedges:\n  - from: multiply\n    to: divide\n    durability: auto\n",
            repository_path("demos/basic/multiply-by-nine.wat").display(),
            repository_path("demos/basic/divide-by-five.wat").display(),
        ),
        std::path::Path::new("."),
        16,
    )
    .expect("workflow should parse")
}

fn resolved_required_plan() -> (BTreeMap<usize, bool>, Vec<AutoResolution>) {
    let resolved = BTreeMap::from([(0, true)]);
    let auto_plan = vec![AutoResolution {
        index: 0,
        required: true,
        profile_id: "test-shape".to_owned(),
        reason: "supplied directly by the test, never read from a profile file".to_owned(),
        profile: Some(DurabilityProfile {
            recompute_us: 4200,
            checkpoint_bytes: 128,
            checkpoint_us: 702,
            samples: 3,
        }),
    }];
    (resolved, auto_plan)
}

#[tokio::test]
async fn a_group_starting_at_a_nonzero_index_executes_and_completes_correctly() {
    let group_one_state = StateFile::new("group-one");
    let group_two_state = StateFile::new("group-two");
    let workflow = two_step_workflow(21);
    let runtime = Runtime::new(Config::default()).expect("runtime should initialize");
    let artifacts = ArtifactStore::memory();
    let (resolved, auto_plan) = resolved_required_plan();

    // no `.kairo/profiles/` file exists anywhere -- if this called `resolve_durability` itself
    // it would fail with `DurabilityProfileMissing`. It must not.
    let first = runtime
        .run_cell_group(
            &workflow,
            group_one_state.path(),
            Some(&artifacts),
            &resolved,
            &auto_plan,
            0,
            21,
            Some(0),
        )
        .await
        .expect("group one should run to its boundary");

    let GroupOutcome::Yielded {
        next_index,
        artifact_hash,
        artifact_backend,
    } = first
    else {
        panic!("group one should yield at its required boundary, got {first:?}");
    };
    assert_eq!(next_index, 1);
    assert_eq!(artifact_backend, artifacts.backend().as_str());

    let handoff = artifacts
        .get(&artifact_hash)
        .await
        .expect("the committed artifact should be fetchable by hash");
    assert_eq!(handoff.value, 189, "21 * 9");

    // group two starts on a completely separate, fresh journal -- simulating a different worker.
    let second = runtime
        .run_cell_group(
            &workflow,
            group_two_state.path(),
            Some(&artifacts),
            &resolved,
            &auto_plan,
            next_index,
            handoff.value,
            None,
        )
        .await
        .expect("group two should run to completion");

    let GroupOutcome::Completed { output } = second else {
        panic!("group two should complete, got {second:?}");
    };
    assert_eq!(output, 37, "189 / 5");
}

#[tokio::test]
async fn a_group_journal_is_independent_of_the_prior_group_journal() {
    let group_one_state = StateFile::new("independent-group-one");
    let group_two_state = StateFile::new("independent-group-two");
    let workflow = two_step_workflow(21);
    let runtime = Runtime::new(Config::default()).expect("runtime should initialize");
    let artifacts = ArtifactStore::memory();
    let (resolved, auto_plan) = resolved_required_plan();

    runtime
        .run_cell_group(
            &workflow,
            group_one_state.path(),
            Some(&artifacts),
            &resolved,
            &auto_plan,
            0,
            21,
            Some(0),
        )
        .await
        .expect("group one should run to its boundary");

    runtime
        .run_cell_group(
            &workflow,
            group_two_state.path(),
            Some(&artifacts),
            &resolved,
            &auto_plan,
            1,
            189,
            None,
        )
        .await
        .expect("group two should run to completion");

    let group_two_inspection =
        inspect_cell(group_two_state.path()).expect("group two's journal should be inspectable");
    assert_eq!(
        group_two_inspection.components.len(),
        1,
        "group two's own journal only ever recorded its own single step, never group one's"
    );
    assert_eq!(group_two_inspection.components[0].name, "divide");
    assert_eq!(group_two_inspection.components[0].input, 189);
}

#[tokio::test]
async fn resuming_the_same_group_journal_after_a_crash_completes_without_redoing_work() {
    let state = StateFile::new("group-resume");
    let workflow = two_step_workflow(21);
    let runtime = Runtime::new(Config::default()).expect("runtime should initialize");
    let artifacts = ArtifactStore::memory();
    let (resolved, auto_plan) = resolved_required_plan();

    let first = runtime
        .run_cell_group(
            &workflow,
            state.path(),
            Some(&artifacts),
            &resolved,
            &auto_plan,
            1,
            189,
            None,
        )
        .await
        .expect("first attempt should complete");

    // simulates the same worker reopening the same group journal after a crash, with the
    // identical resume parameters the control plane would have kept on file.
    let second = runtime
        .run_cell_group(
            &workflow,
            state.path(),
            Some(&artifacts),
            &resolved,
            &auto_plan,
            1,
            189,
            None,
        )
        .await
        .expect("resumed attempt should complete without redoing work");

    let (
        GroupOutcome::Completed {
            output: first_output,
        },
        GroupOutcome::Completed {
            output: second_output,
        },
    ) = (first, second)
    else {
        panic!("both attempts should complete");
    };
    assert_eq!(first_output, second_output);
    assert_eq!(event_count(state.path(), "component_started"), 1);
}

#[tokio::test]
async fn reopening_a_group_journal_with_a_different_start_index_is_rejected() {
    let state = StateFile::new("group-mismatch");
    let workflow = two_step_workflow(21);
    let runtime = Runtime::new(Config::default()).expect("runtime should initialize");
    let artifacts = ArtifactStore::memory();
    let (resolved, auto_plan) = resolved_required_plan();

    runtime
        .run_cell_group(
            &workflow,
            state.path(),
            Some(&artifacts),
            &resolved,
            &auto_plan,
            1,
            189,
            None,
        )
        .await
        .expect("first attempt should complete");

    let error = runtime
        .run_cell_group(
            &workflow,
            state.path(),
            Some(&artifacts),
            &resolved,
            &auto_plan,
            0,
            21,
            Some(0),
        )
        .await
        .expect_err("a mismatched start index must never silently reinterpret the journal");

    assert!(matches!(
        error,
        RuntimeError::Journal {
            source: kairo_runtime::JournalError::WorkflowChanged,
            ..
        }
    ));
}

fn event_count(path: &std::path::Path, kind: &str) -> usize {
    let connection = rusqlite::Connection::open(path).expect("journal should open");
    connection
        .query_row(
            &format!("SELECT COUNT(*) FROM events WHERE kind = '{kind}'"),
            [],
            |row| row.get::<_, i64>(0),
        )
        .expect("count should query") as usize
}
