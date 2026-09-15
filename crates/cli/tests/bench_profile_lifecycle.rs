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
        let mut kill = kairo();
        kill.current_dir(&self.0).arg("down");
        let _ = kill.output();
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn write_workflow(directory: &Directory, name: &str, steps: &[(&str, PathBuf)]) -> PathBuf {
    let step_yaml = steps
        .iter()
        .map(|(step, component)| {
            format!("  - name: {step}\n    component: {}\n", component.display())
        })
        .collect::<String>();
    let edge_yaml = steps
        .windows(2)
        .map(|pair| {
            format!(
                "  - from: {}\n    to: {}\n    durability: auto\n",
                pair[0].0, pair[1].0
            )
        })
        .collect::<String>();
    let path = directory.0.join(format!("{name}.yaml"));
    fs::write(
        &path,
        format!("workflow: {name}\ninput: 100\nsteps:\n{step_yaml}edges:\n{edge_yaml}"),
    )
    .expect("workflow should write");
    path
}

fn stale_bench_profile_runs(directory: &Directory) -> Vec<String> {
    let path = directory.0.join(".kairo/control-state.json");
    let Ok(bytes) = fs::read(&path) else {
        return Vec::new();
    };
    let state: serde_json::Value =
        serde_json::from_slice(&bytes).expect("control state should be valid JSON");
    state["runs"]
        .as_object()
        .into_iter()
        .flat_map(|runs| runs.keys())
        .filter(|id| id.starts_with("bench-profile-"))
        .cloned()
        .collect()
}

#[test]
fn succeeds_from_a_clean_project() {
    let directory = Directory::new("bench-profile-clean");
    let workflow = write_workflow(
        &directory,
        "two-step",
        &[
            (
                "multiply",
                repository_path("demos/basic/multiply-by-nine.wat"),
            ),
            ("divide", repository_path("demos/basic/divide-by-five.wat")),
        ],
    );

    let output = kairo()
        .current_dir(&directory.0)
        .args(["bench", "run"])
        .arg(&workflow)
        .args(["--repetitions", "2", "--profile"])
        .output()
        .expect("bench profile should run");
    assert!(output.status.success(), "{output:?}");

    let profiles: Vec<_> = fs::read_dir(directory.0.join(".kairo/profiles"))
        .expect("profiles directory should exist")
        .collect();
    assert_eq!(profiles.len(), 1, "expected exactly one profile written");
    let report: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(profiles.into_iter().next().unwrap().unwrap().path()).unwrap(),
    )
    .expect("profile should be valid JSON");
    let edge = &report["edges"]["multiply"];
    assert!(edge["samples"].as_u64().unwrap() > 0, "{report}");
    assert!(edge["checkpoint_bytes"].as_u64().is_some(), "{report}");
}

#[test]
fn multiple_internal_passes_and_steps_do_not_collide() {
    let directory = Directory::new("bench-profile-multi-edge");
    let workflow = write_workflow(
        &directory,
        "three-step",
        &[
            (
                "multiply",
                repository_path("demos/basic/multiply-by-nine.wat"),
            ),
            ("divide", repository_path("demos/basic/divide-by-five.wat")),
            ("add", repository_path("demos/basic/add-thirty-two.wat")),
        ],
    );

    // two auto edges, three repetitions each, two measurement passes per edge -- twelve internal
    // `bench-profile-N` submissions reusing names 0..2 four times over, in one invocation.
    let output = kairo()
        .current_dir(&directory.0)
        .args(["bench", "run"])
        .arg(&workflow)
        .args(["--repetitions", "3", "--profile"])
        .output()
        .expect("bench profile should run");
    assert!(output.status.success(), "{output:?}");

    let report_path = fs::read_dir(directory.0.join(".kairo/profiles"))
        .expect("profiles directory should exist")
        .next()
        .expect("a profile should exist")
        .unwrap()
        .path();
    let report: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(report_path).unwrap()).unwrap();
    assert!(report["edges"]["multiply"]["samples"].as_u64().unwrap() > 0);
    assert!(report["edges"]["divide"]["samples"].as_u64().unwrap() > 0);
    assert!(stale_bench_profile_runs(&directory).is_empty());
}

#[test]
fn a_second_profiling_invocation_in_the_same_project_also_succeeds() {
    let directory = Directory::new("bench-profile-repeat");
    let workflow = write_workflow(
        &directory,
        "two-step",
        &[
            (
                "multiply",
                repository_path("demos/basic/multiply-by-nine.wat"),
            ),
            ("divide", repository_path("demos/basic/divide-by-five.wat")),
        ],
    );

    for attempt in 0..2 {
        let output = kairo()
            .current_dir(&directory.0)
            .args(["bench", "run"])
            .arg(&workflow)
            .args(["--repetitions", "2", "--profile"])
            .output()
            .expect("bench profile should run");
        assert!(
            output.status.success(),
            "profiling attempt {attempt} failed: {output:?}"
        );
    }
    assert!(stale_bench_profile_runs(&directory).is_empty());
}

#[test]
fn no_stale_bench_profile_runs_remain_after_profiling() {
    let directory = Directory::new("bench-profile-no-residue");
    let workflow = write_workflow(
        &directory,
        "two-step",
        &[
            (
                "multiply",
                repository_path("demos/basic/multiply-by-nine.wat"),
            ),
            ("divide", repository_path("demos/basic/divide-by-five.wat")),
        ],
    );

    let output = kairo()
        .current_dir(&directory.0)
        .args(["bench", "run"])
        .arg(&workflow)
        .args(["--repetitions", "2", "--profile"])
        .output()
        .expect("bench profile should run");
    assert!(output.status.success(), "{output:?}");
    assert!(
        stale_bench_profile_runs(&directory).is_empty(),
        "internal profiling runs must not remain as ordinary user-visible control state"
    );
}

#[test]
fn a_failure_partway_still_cleans_up() {
    let directory = Directory::new("bench-profile-failure");
    let failing = write_workflow(
        &directory,
        "boom",
        &[
            ("start", repository_path("demos/basic/multiply-by-nine.wat")),
            (
                "trap",
                repository_path("crates/cli/tests/fixtures/always-traps.wat"),
            ),
        ],
    );

    let failed = kairo()
        .current_dir(&directory.0)
        .args(["bench", "run"])
        .arg(&failing)
        .args(["--repetitions", "2", "--profile"])
        .output()
        .expect("bench profile should run");
    assert!(!failed.status.success(), "{failed:?}");
    assert!(
        stale_bench_profile_runs(&directory).is_empty(),
        "a failed profiling attempt must not leave any run registered either"
    );

    // the failed attempt must not have left the control service (or its worker pool) in a state
    // that blocks a completely unrelated, real profiling run afterward.
    let working = write_workflow(
        &directory,
        "two-step",
        &[
            (
                "multiply",
                repository_path("demos/basic/multiply-by-nine.wat"),
            ),
            ("divide", repository_path("demos/basic/divide-by-five.wat")),
        ],
    );
    let succeeded = kairo()
        .current_dir(&directory.0)
        .args(["bench", "run"])
        .arg(&working)
        .args(["--repetitions", "2", "--profile"])
        .output()
        .expect("bench profile should run");
    assert!(succeeded.status.success(), "{succeeded:?}");
    assert!(stale_bench_profile_runs(&directory).is_empty());
}
