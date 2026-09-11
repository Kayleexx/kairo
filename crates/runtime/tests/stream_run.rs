#![allow(clippy::expect_used)]

use std::{path::PathBuf, time::Duration};

use kairo_runtime::{StreamMetrics, StreamRun, StreamRunStatus, StreamValue, inspect_stream_run};
use rusqlite::Connection;

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
    run.complete_with_values(
        Duration::from_micros(42),
        3,
        6_350,
        StreamMetrics {
            source_bytes: 183,
            consumed_bytes: 183,
            largest_batch_bytes: 64,
            materialized_bytes: 0,
        },
        None,
        &[
            StreamValue {
                name: "records".to_owned(),
                value: 3,
            },
            StreamValue {
                name: "total-cents".to_owned(),
                value: 6_350,
            },
        ],
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
        inspection.values,
        [
            StreamValue {
                name: "records".to_owned(),
                value: 3,
            },
            StreamValue {
                name: "total-cents".to_owned(),
                value: 6_350,
            },
        ]
    );
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
fn persists_input_provenance_without_an_absolute_path() {
    let path = temporary("provenance");
    let mut run = StreamRun::start_with_provenance(
        &path,
        "doc",
        "records.csv",
        "user",
        &["jsonl".to_owned(), "csv".to_owned()],
        &["summarize".to_owned()],
        Some(("records", "total-cents")),
    )
    .expect("stream state should start");
    run.complete_with_hash(
        Duration::from_micros(1),
        1,
        20,
        StreamMetrics::default(),
        Some("sha256:1234"),
    )
    .expect("stream state should complete");

    let inspection = inspect_stream_run(&path)
        .expect("stream state should be readable")
        .expect("stream marker should exist");
    assert_eq!(inspection.input, "records.csv");
    assert_eq!(inspection.input_source.as_deref(), Some("user"));
    assert_eq!(inspection.input_hash.as_deref(), Some("sha256:1234"));
    assert_eq!(inspection.input_accepts, ["jsonl", "csv"]);
    cleanup(&path);
}

#[test]
fn reads_stream_state_created_before_input_provenance() {
    let path = temporary("legacy");
    let connection = Connection::open(&path).expect("legacy database should open");
    connection.execute_batch("CREATE TABLE stream_run(id INTEGER PRIMARY KEY, workflow TEXT NOT NULL, input TEXT NOT NULL, status TEXT NOT NULL, error TEXT, duration_us INTEGER, high INTEGER, low INTEGER, high_label TEXT, low_label TEXT, source_bytes INTEGER, consumed_bytes INTEGER, largest_batch_bytes INTEGER, materialized_bytes INTEGER); INSERT INTO stream_run(id,workflow,input,status) VALUES(1,'video','clip.y4m','running'); CREATE TABLE stream_steps(step_index INTEGER PRIMARY KEY, name TEXT NOT NULL); INSERT INTO stream_steps VALUES(0,'analyze');").expect("legacy state should be written");
    drop(connection);

    let inspection = inspect_stream_run(&path)
        .expect("legacy stream state should be readable")
        .expect("legacy marker should exist");
    assert_eq!(inspection.input_source, None);
    assert_eq!(inspection.input_hash, None);
    assert!(inspection.input_accepts.is_empty());
    assert!(inspection.values.is_empty());
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
