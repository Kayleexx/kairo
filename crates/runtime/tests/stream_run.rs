#![allow(clippy::expect_used)]

use std::{path::PathBuf, time::Duration};

use kairo_runtime::{StreamMetrics, StreamRun, StreamRunStatus, inspect_stream_run};

#[test]
fn persists_a_bounded_stream_summary() {
    let path = temporary("complete");
    let mut run = StreamRun::start(
        &path,
        "documents",
        PathBuf::from("records.jsonl").as_path(),
        &["normalize".to_owned(), "aggregate".to_owned()],
        Some(("records", "total-cents")),
    )
    .expect("stream state should start");
    run.complete(
        Duration::from_micros(42),
        3,
        6_350,
        StreamMetrics {
            source_bytes: 183,
            consumed_bytes: 183,
            largest_batch_bytes: 64,
            materialized_bytes: 0,
        },
    )
    .expect("stream state should complete");

    let inspection = inspect_stream_run(&path)
        .expect("stream state should be readable")
        .expect("stream marker should exist");
    assert_eq!(inspection.status, StreamRunStatus::Completed);
    assert_eq!(inspection.high, Some(3));
    assert_eq!(inspection.low, Some(6_350));
    assert_eq!(inspection.steps, ["normalize", "aggregate"]);
    assert_eq!(
        inspection
            .metrics
            .expect("metrics should exist")
            .source_bytes,
        183
    );
    cleanup(&path);
}

#[test]
fn bounds_a_recorded_failure() {
    let path = temporary("failure");
    let mut run = StreamRun::start(
        &path,
        "documents",
        PathBuf::from("records.jsonl").as_path(),
        &["aggregate".to_owned()],
        None,
    )
    .expect("stream state should start");
    run.fail(&"x".repeat(2_000))
        .expect("failure should be recorded");

    let inspection = inspect_stream_run(&path)
        .expect("stream state should be readable")
        .expect("stream marker should exist");
    assert_eq!(
        inspection.status,
        StreamRunStatus::Failed("x".repeat(1_024))
    );
    cleanup(&path);
}

fn temporary(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("kairo-{name}-{}.db", std::process::id()))
}

fn cleanup(path: &PathBuf) {
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
}
