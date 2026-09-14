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

#[test]
fn produces_a_parseable_report_with_real_fields() {
    let directory = Directory::new("bench-report");
    let output = kairo()
        .current_dir(&directory.0)
        .args(["bench", "run"])
        .arg(repository_path("demos/checkout/workflow.yaml"))
        .args(["--warmups", "1", "--repetitions", "3"])
        .output()
        .expect("bench should run");
    assert!(output.status.success(), "{output:?}");

    let reports: Vec<_> = fs::read_dir(directory.0.join(".kairo/benchmarks"))
        .expect("benchmarks directory should exist")
        .collect();
    assert_eq!(reports.len(), 1);
    let report_path = reports.into_iter().next().unwrap().unwrap().path();
    let report: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&report_path).unwrap())
            .expect("report should be valid JSON");

    assert_eq!(report["failures"].as_array().unwrap().len(), 0);
    let samples = report["raw_samples"].as_array().unwrap();
    // warmups are excluded from raw_samples -- only the 3 timed repetitions land here.
    assert_eq!(samples.len(), 3);
    for sample in samples {
        assert_eq!(sample["output"], 3207);
        assert!(sample["workflow_duration_us"].as_u64().unwrap() > 0);
        assert!(sample["wall_ms"].is_u64());
    }
    assert_eq!(report["summary"]["successes"], 3);
    assert!(report["summary"]["wall_ms_p50"].as_u64().unwrap() > 0);
}

#[test]
fn failing_attempts_land_in_failures_not_summary() {
    let directory = Directory::new("bench-failure");
    let workflow = directory.0.join("runaway.yaml");
    fs::write(
        &workflow,
        format!(
            "workflow: bench-runaway\ninput: 21\nsteps:\n  - name: runaway\n    component: {}\nedges: []\n",
            repository_path("components/runtime/runaway.wat").display(),
        ),
    )
    .expect("workflow should be written");

    let output = kairo()
        .current_dir(&directory.0)
        .args(["bench", "run"])
        .arg(&workflow)
        .args(["--warmups", "0", "--repetitions", "2"])
        .output()
        .expect("bench should run");
    assert!(output.status.success(), "{output:?}");

    let report_path = fs::read_dir(directory.0.join(".kairo/benchmarks"))
        .expect("benchmarks directory should exist")
        .next()
        .expect("a report should be written")
        .unwrap()
        .path();
    let report: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&report_path).unwrap())
            .expect("report should be valid JSON");

    assert!(report["summary"].is_null());
    assert_eq!(report["raw_samples"].as_array().unwrap().len(), 0);
    let failures = report["failures"].as_array().unwrap();
    assert_eq!(failures.len(), 2);
    for failure in failures {
        assert!(failure["message"].as_str().unwrap().contains("fuel"));
    }
}

#[test]
fn refuses_to_silently_overwrite_an_existing_report() {
    let directory = Directory::new("bench-overwrite");
    let report_path = PathBuf::from(".kairo/benchmarks/fixed.json");
    let run = |directory: &Directory| {
        kairo()
            .current_dir(&directory.0)
            .args(["bench", "run"])
            .arg(repository_path("demos/checkout/workflow.yaml"))
            .args(["--warmups", "0", "--repetitions", "1", "--output"])
            .arg(&report_path)
            .output()
            .expect("bench should run")
    };

    let first = run(&directory);
    assert!(first.status.success(), "{first:?}");
    let original = fs::read_to_string(directory.0.join(&report_path)).unwrap();

    let second = run(&directory);
    assert!(!second.status.success());
    assert!(
        String::from_utf8_lossy(&second.stderr).contains("already exists"),
        "{second:?}"
    );
    let unchanged = fs::read_to_string(directory.0.join(&report_path)).unwrap();
    assert_eq!(
        original, unchanged,
        "a failed second write must not touch the existing report"
    );
}
