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
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn write_two_step_auto_workflow(directory: &Directory, name: &str) -> PathBuf {
    let path = directory.0.join(format!("{name}.yaml"));
    fs::write(
        &path,
        format!(
            "workflow: {name}\ninput: 100\nsteps:\n  - name: multiply\n    component: {}\n  - name: divide\n    component: {}\nedges:\n  - from: multiply\n    to: divide\n    durability: auto\n",
            repository_path("demos/basic/multiply-by-nine.wat").display(),
            repository_path("demos/basic/divide-by-five.wat").display(),
        ),
    )
    .expect("workflow should write");
    path
}

#[test]
fn workflow_show_matches_the_legacy_workflows_path_form() {
    let directory = Directory::new("workflow-show");
    let workflow = write_two_step_auto_workflow(&directory, "two-step");

    let show = kairo()
        .current_dir(&directory.0)
        .args(["workflow", "show"])
        .arg(&workflow)
        .output()
        .expect("workflow show should run");
    assert!(show.status.success(), "{show:?}");

    let legacy = kairo()
        .current_dir(&directory.0)
        .arg("workflows")
        .arg(&workflow)
        .output()
        .expect("workflows <path> should run");
    assert!(legacy.status.success(), "{legacy:?}");

    assert_eq!(show.stdout, legacy.stdout);
}

#[test]
fn workflow_profile_matches_the_legacy_bench_run_profile_form() {
    let directory = Directory::new("workflow-profile");
    let workflow = write_two_step_auto_workflow(&directory, "two-step");

    let profile = kairo()
        .current_dir(&directory.0)
        .args(["workflow", "profile"])
        .arg(&workflow)
        .args(["--repetitions", "2"])
        .output()
        .expect("workflow profile should run");
    assert!(profile.status.success(), "{profile:?}");

    let profiles: Vec<_> = fs::read_dir(directory.0.join(".kairo/profiles"))
        .expect("profiles directory should exist")
        .collect();
    assert_eq!(profiles.len(), 1, "expected exactly one profile written");

    // the resolved decision must actually be visible afterward, through the same command
    // sequence a user would type -- not just a file existing on disk.
    let show = kairo()
        .current_dir(&directory.0)
        .args(["workflow", "show"])
        .arg(&workflow)
        .output()
        .expect("workflow show should run");
    assert!(show.status.success(), "{show:?}");
    let rendered = String::from_utf8_lossy(&show.stdout);
    assert!(
        rendered.contains("recompute") && rendered.contains("checkpoint"),
        "resolved auto decision should show its real measured reason: {rendered}"
    );
}

#[test]
fn bench_run_profile_still_works_as_a_backwards_compatible_alias() {
    let directory = Directory::new("bench-profile-alias");
    let workflow = write_two_step_auto_workflow(&directory, "two-step");

    let output = kairo()
        .current_dir(&directory.0)
        .args(["bench", "run"])
        .arg(&workflow)
        .args(["--repetitions", "2", "--profile"])
        .output()
        .expect("bench run --profile should run");
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        fs::read_dir(directory.0.join(".kairo/profiles"))
            .expect("profiles directory should exist")
            .count(),
        1
    );
}
