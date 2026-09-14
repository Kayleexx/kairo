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

struct Fixture {
    directory: PathBuf,
    workflow: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
        let directory =
            std::env::temp_dir().join(format!("kairo-prune-{}-{sequence}", process::id()));
        fs::create_dir(&directory).expect("fixture directory should be created");
        let workflow = directory.join("workflow.yaml");
        fs::write(
            &workflow,
            format!(
                "workflow: prune-test\ninput: 20\nsteps:\n  - name: multiply\n    component: {}\nedges: []\n",
                repository_path("demos/basic/multiply-by-nine.wat").display(),
            ),
        )
        .expect("workflow should be written");
        Self {
            directory,
            workflow,
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

#[test]
fn dry_run_reports_without_deleting() {
    let fixture = Fixture::new();
    let output = kairo()
        .current_dir(&fixture.directory)
        .arg("run")
        .arg(&fixture.workflow)
        .arg("--run")
        .arg("completed-run")
        .output()
        .expect("kairo should start");
    assert!(output.status.success(), "{output:?}");
    let journal = fixture.directory.join(".kairo").join("completed-run.db");
    assert!(journal.exists());

    let output = kairo()
        .current_dir(&fixture.directory)
        .arg("prune")
        .output()
        .expect("kairo should start");
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8(output.stdout).expect("output should be UTF-8");
    assert!(stdout.contains("would remove"), "got: {stdout}");
    assert!(journal.exists(), "dry run must not delete anything");
}

#[test]
fn yes_removes_completed_runs_and_their_siblings() {
    let fixture = Fixture::new();
    let output = kairo()
        .current_dir(&fixture.directory)
        .arg("run")
        .arg(&fixture.workflow)
        .arg("--run")
        .arg("completed-run")
        .output()
        .expect("kairo should start");
    assert!(output.status.success(), "{output:?}");
    let journal = fixture.directory.join(".kairo").join("completed-run.db");
    let lock = fixture.directory.join(".kairo").join("completed-run.lock");
    assert!(journal.exists());

    let output = kairo()
        .current_dir(&fixture.directory)
        .arg("prune")
        .arg("--yes")
        .output()
        .expect("kairo should start");
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8(output.stdout).expect("output should be UTF-8");
    assert!(stdout.contains("removed"), "got: {stdout}");
    assert!(!journal.exists(), "journal should be removed");
    assert!(!lock.exists(), "lock file should be removed");
}

#[test]
fn workflow_filter_skips_other_workflows() {
    let fixture = Fixture::new();
    kairo()
        .current_dir(&fixture.directory)
        .arg("run")
        .arg(&fixture.workflow)
        .arg("--run")
        .arg("completed-run")
        .output()
        .expect("kairo should start");
    let journal = fixture.directory.join(".kairo").join("completed-run.db");
    assert!(journal.exists());

    let output = kairo()
        .current_dir(&fixture.directory)
        .arg("prune")
        .arg("--yes")
        .arg("--workflow")
        .arg("some-other-workflow")
        .output()
        .expect("kairo should start");
    assert!(output.status.success(), "{output:?}");
    assert!(
        journal.exists(),
        "an unrelated workflow filter must not delete this run"
    );
}
