#![allow(clippy::expect_used, clippy::unwrap_used)]

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
    fn new(name: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("kairo-{name}-{}-{sequence}", process::id()));
        fs::create_dir(&path).expect("fixture directory should be created");
        Self(path)
    }
}

impl Drop for Directory {
    fn drop(&mut self) {
        let _ = kairo().current_dir(&self.0).arg("down").output();
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// a `required`-edge workflow, no service up, no `--watch`, must still go through managed
/// execution automatically.
#[test]
fn a_durable_workflow_auto_starts_a_local_service_and_completes() {
    let directory = Directory::new("auto-managed-durable");
    let output = kairo()
        .current_dir(&directory.0)
        .arg("run")
        .arg(repository_path("demos/checkout/workflow.yaml"))
        .args(["--run", "auto-managed-run"])
        .output()
        .expect("run should complete");
    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, b"3207\n");

    // a real control plane ran this, not a direct in-process execution -- its endpoint file is
    // cleaned up once the ephemeral service stops, but its durable run history is not.
    assert!(directory.0.join(".kairo/control-state.json").exists());
    assert!(directory.0.join(".kairo/auto-managed-run.db").exists());
}

/// checkout-settlement's required edge splits it into two ExecutionGroups -- exercises the
/// multi-journal aggregation across `inspect`/`runs`/`prune`.
#[test]
fn a_multi_group_run_is_inspected_and_pruned_as_one_run() {
    let directory = Directory::new("auto-managed-groups");
    let run = kairo()
        .current_dir(&directory.0)
        .arg("run")
        .arg(repository_path("demos/checkout/workflow.yaml"))
        .args(["--run", "multi-group-run"])
        .output()
        .expect("run should complete");
    assert!(run.status.success(), "{run:?}");

    assert!(
        directory
            .0
            .join(".kairo/multi-group-run.group-3.db")
            .exists(),
        "checkout-settlement's required edge should split it into a second ExecutionGroup"
    );

    let inspection = kairo()
        .current_dir(&directory.0)
        .args(["inspect", "multi-group-run"])
        .output()
        .expect("inspect should run");
    assert!(inspection.status.success(), "{inspection:?}");
    let rendered = String::from_utf8_lossy(&inspection.stdout);
    assert!(rendered.contains("state · completed"), "{rendered}");
    assert!(rendered.contains("output · 3207"), "{rendered}");
    for component in ["apply-discount", "add-tax", "add-handling", "add-shipping"] {
        assert!(rendered.contains(component), "{rendered}");
    }

    let runs = kairo()
        .current_dir(&directory.0)
        .arg("runs")
        .output()
        .expect("runs should run");
    assert!(runs.status.success(), "{runs:?}");
    let runs_stdout = String::from_utf8_lossy(&runs.stdout);
    assert!(runs_stdout.contains("runs · 1"), "{runs_stdout}");
    assert!(
        !runs_stdout.contains("group-3"),
        "the group's own journal must never appear as its own run: {runs_stdout}"
    );

    let prune = kairo()
        .current_dir(&directory.0)
        .args(["prune", "--yes"])
        .output()
        .expect("prune should run");
    assert!(prune.status.success(), "{prune:?}");
    assert!(!directory.0.join(".kairo/multi-group-run.db").exists());
    assert!(
        !directory
            .0
            .join(".kairo/multi-group-run.group-3.db")
            .exists(),
        "prune must also remove the group's own journal, not just the base one"
    );
}

/// the bundled slow/chaos reference demo: a real, non-fixture workflow with a required edge,
/// runnable without knowing internal WAT test-fixture paths.
#[test]
fn the_locality_reference_demo_runs_and_splits_into_groups() {
    let directory = Directory::new("locality-reference-demo");
    let output = kairo()
        .current_dir(&directory.0)
        .arg("--allow-console")
        .arg("run")
        .arg(repository_path("demos/locality/workflow.yaml"))
        .args(["--run", "locality-demo-run"])
        .output()
        .expect("run should complete");
    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, b"22\n");

    assert!(
        directory
            .0
            .join(".kairo/locality-demo-run.group-2.db")
            .exists(),
        "the demo's required edge should split it into a second ExecutionGroup"
    );
}

/// `bench --profile` measures a `durability: auto` edge by running a `required`-forced variant
/// through the real control plane -- which can itself split into ExecutionGroups. This must be
/// read through the same aggregated view as `inspect`/`bench run`, not the base journal alone.
#[test]
fn profiling_an_auto_edge_survives_a_multi_group_profiling_run() {
    let directory = Directory::new("auto-profile-groups");
    fs::write(
        directory.0.join("workflow.yaml"),
        format!(
            "workflow: auto-profile-demo\ninput: 2500\n\nsteps:\n  - name: apply-discount\n    component: {}\n  - name: add-tax\n    component: {}\n\nedges:\n  - from: apply-discount\n    to: add-tax\n    durability: auto\n",
            repository_path("demos/checkout/apply-discount.wat").display(),
            repository_path("demos/checkout/add-tax.wat").display(),
        ),
    )
    .expect("scratch workflow should be written");

    let profile = kairo()
        .current_dir(&directory.0)
        .args(["bench", "run", "workflow.yaml", "--profile"])
        .output()
        .expect("profile should run");
    assert!(profile.status.success(), "{profile:?}");

    let run = kairo()
        .current_dir(&directory.0)
        .arg("run")
        .arg("workflow.yaml")
        .output()
        .expect("run should complete now that a profile exists");
    assert!(run.status.success(), "{run:?}");
}

/// a fully ephemeral workflow must stay on the fast path -- no control plane at all.
#[test]
fn a_fully_ephemeral_workflow_never_starts_a_service() {
    let directory = Directory::new("auto-managed-ephemeral");
    let output = kairo()
        .current_dir(&directory.0)
        .arg("run")
        .arg(repository_path("demos/basic/workflow.yaml"))
        .output()
        .expect("run should complete");
    assert!(output.status.success(), "{output:?}");

    assert!(
        !directory.0.join(".kairo/control.json").exists(),
        "a fully ephemeral workflow must never bring up a local control plane"
    );
}
