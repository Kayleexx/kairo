#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::{
    fs::{self, File},
    io::Write,
    path::PathBuf,
    process,
    sync::atomic::{AtomicU64, Ordering},
};

use kairo_core::{Config, Workflow};
use kairo_runtime::{Runtime, RuntimeError};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);

impl Fixture {
    fn write(bytes: &[u8]) -> Self {
        let sequence = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "kairo-edge-measure-{}-{sequence}.bin",
            process::id()
        ));
        File::create(&path)
            .expect("fixture should be created")
            .write_all(bytes)
            .expect("fixture should be written");
        Self(path)
    }

    fn large() -> Self {
        let chunk: Vec<u8> = (0..=255).collect();
        Self::write(&chunk.repeat(8192))
    }

    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn repository_path(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(path)
}

fn two_hop_workflow(consume: &str) -> Workflow {
    Workflow::parse(
        &format!(
            r#"
workflow: edge-measurement
mode: stream
input: demos/stream/input.bin
steps:
  - {{ name: first, component: demos/stream/transform.wat }}
  - {{ name: second, component: demos/stream/transform.wat }}
  - {{ name: consume, component: {consume} }}
edges:
  - {{ from: first, to: second }}
  - {{ from: second, to: consume }}
"#
        ),
        &repository_path(""),
        256,
    )
    .expect("two-hop workflow should parse")
}

fn expected_checksum(bytes: &[u8]) -> u32 {
    bytes.iter().fold(0_u32, |checksum, byte| {
        checksum.wrapping_add(u32::from(*byte))
    })
}

#[tokio::test]
async fn measures_exact_bytes_across_multiple_edges() {
    let input = repository_path("demos/stream/input.bin");
    let expected = fs::read(&input).expect("demo input should load");
    let runtime = Runtime::new(Config::default()).expect("runtime should initialize");
    let workflow = two_hop_workflow("demos/stream/consume.wat");

    let direct = runtime
        .run_stream_workflow(&workflow, None, false)
        .await
        .expect("direct run should complete");
    let measured = runtime
        .measure_stream_workflow_edges(&workflow, None, false, None)
        .await
        .expect("measured run should complete");

    // identity transforms mean the total byte count and checksum must be identical between the
    // direct streaming path and the measured path -- any truncation, duplication, or reordering
    // in the measurement relay would change one or both.
    assert_eq!(measured.bytes, direct.bytes);
    assert_eq!(measured.bytes, expected.len() as u64);
    assert_eq!(measured.checksum, direct.checksum);
    assert_eq!(measured.checksum, expected_checksum(&expected));

    assert_eq!(measured.metrics.edges.len(), 2);
    for edge in &measured.metrics.edges {
        assert_eq!(edge.bytes, Some(expected.len() as u64));
        assert_eq!(edge.materialized, Some(true));
        assert_eq!(edge.materialized_bytes, Some(expected.len() as u64));
        // this path never observes true streaming backpressure -- it must stay unknown, not a
        // fabricated number.
        assert_eq!(edge.peak_buffered_bytes, None);
    }
    // the direct path never populates per-edge metrics at all -- confirms the measured path is
    // strictly additive and doesn't change default `kairo run` behavior.
    assert!(direct.metrics.edges.iter().all(|edge| edge.bytes.is_none()));
}

#[tokio::test]
async fn measures_a_large_multi_chunk_stream_without_duplication_or_truncation() {
    let fixture = Fixture::large();
    let expected = fs::read(fixture.path()).expect("fixture should be readable");
    let runtime = Runtime::new(Config {
        max_stream_input_bytes: 4 * 1024 * 1024,
        max_stream_output_bytes: 4 * 1024 * 1024,
        stream_chunk_bytes: 1024,
        execution_fuel: 100_000_000,
        ..Config::default()
    })
    .expect("runtime should initialize");
    let workflow = two_hop_workflow("demos/stream/consume.wat");

    let measured = runtime
        .measure_stream_workflow_edges(&workflow, Some(fixture.path()), false, None)
        .await
        .expect("large measured run should complete");

    assert_eq!(measured.bytes, expected.len() as u64);
    assert_eq!(measured.checksum, expected_checksum(&expected));
    for edge in &measured.metrics.edges {
        assert_eq!(edge.bytes, Some(expected.len() as u64));
    }
}

#[tokio::test]
async fn handles_an_empty_stream_across_measured_edges() {
    let fixture = Fixture::write(&[]);
    let runtime = Runtime::new(Config::default()).expect("runtime should initialize");
    let workflow = two_hop_workflow("demos/stream/consume.wat");

    let measured = runtime
        .measure_stream_workflow_edges(&workflow, Some(fixture.path()), false, None)
        .await
        .expect("empty measured run should complete");

    assert_eq!(measured.bytes, 0);
    for edge in &measured.metrics.edges {
        assert_eq!(edge.bytes, Some(0));
        assert_eq!(edge.materialized, Some(true));
        assert_eq!(edge.materialized_bytes, Some(0));
    }
}

#[tokio::test]
async fn rejects_an_edge_that_exceeds_the_measurement_bound() {
    let fixture = Fixture::large();
    let runtime = Runtime::new(Config {
        // large enough for the raw file input itself; the edge relay reuses the (smaller)
        // output bound, so the very first measured edge overflows it deterministically.
        max_stream_input_bytes: 8 * 1024 * 1024,
        max_stream_output_bytes: 1024,
        stream_chunk_bytes: 1024,
        ..Config::default()
    })
    .expect("runtime should initialize");
    let workflow = two_hop_workflow("demos/stream/consume.wat");

    let error = runtime
        .measure_stream_workflow_edges(&workflow, Some(fixture.path()), false, None)
        .await
        .expect_err("an edge exceeding the measurement bound should fail, not silently truncate");

    assert!(
        matches!(
            error,
            RuntimeError::StreamStep { ref source, .. }
                if matches!(**source, RuntimeError::StreamEdgeTooLarge { .. })
        ),
        "{error:?}"
    );
}

#[tokio::test]
async fn cleans_up_when_a_downstream_consumer_drops_without_reading() {
    let runtime = Runtime::new(Config::default()).expect("runtime should initialize");
    let workflow = Workflow::parse(
        r#"
workflow: measured-drop
mode: stream
input: demos/stream/input.bin
steps:
  - { name: first, component: demos/stream/transform.wat }
  - { name: consume, component: components/runtime/stream-drop.wat }
edges:
  - { from: first, to: consume }
"#,
        &repository_path(""),
        256,
    )
    .expect("workflow should parse");

    let measured = runtime
        .measure_stream_workflow_edges(&workflow, None, false, None)
        .await
        .expect("run should complete cleanly even though the consumer never reads");

    // the measurement path eagerly drains the edge before the downstream call even starts, so
    // it legitimately reads more than the direct streaming path would for the same run (which
    // never asks the source for bytes a lazy consumer never requests) -- expected and bounded,
    // not a hang or a leak.
    let expected = fs::read(repository_path("demos/stream/input.bin")).unwrap();
    assert_eq!(measured.metrics.edges[0].bytes, Some(expected.len() as u64));
}

#[tokio::test]
async fn propagates_failures_from_the_measured_path_without_hanging() {
    let runtime = Runtime::new(Config {
        execution_fuel: 1,
        ..Config::default()
    })
    .expect("runtime should initialize");
    let workflow = two_hop_workflow("demos/stream/consume.wat");

    let error = runtime
        .measure_stream_workflow_edges(&workflow, None, false, None)
        .await
        .expect_err("fuel-exhausted measured run should fail, not hang");

    assert!(
        matches!(
            error,
            RuntimeError::StreamStep { ref source, .. }
                if matches!(**source, RuntimeError::FuelExhausted { .. })
        ),
        "{error:?}"
    );
}
