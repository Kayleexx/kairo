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
fn produces_a_report_with_real_stream_bytes_and_edges() {
    let directory = Directory::new("bench-stream-report");
    let output = kairo()
        .current_dir(&directory.0)
        .args(["bench", "run"])
        .arg(repository_path("demos/stream/workflow.yaml"))
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

    assert_eq!(report["failures"].as_array().unwrap().len(), 0);
    let samples = report["raw_samples"].as_array().unwrap();
    assert_eq!(samples.len(), 2);
    let expected_bytes = fs::metadata(repository_path("demos/stream/input.bin"))
        .unwrap()
        .len();
    for sample in samples {
        assert!(sample["output"].is_null());
        assert_eq!(sample["stream_bytes"], expected_bytes);
        let edges = sample["edges"].as_array().unwrap();
        assert_eq!(edges.len(), 1, "the demo workflow has exactly one edge");
        assert_eq!(edges[0]["name"], "transform -> consume");
        // the direct streaming path never populates per-edge bytes -- only the materialize
        // measurement path (Slice 13.3) does, and `kairo bench` doesn't opt into it here.
        assert!(edges[0]["bytes"].is_null());
    }
}
