#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::{
    fs::{self, File},
    io::Write,
    path::PathBuf,
    process,
    sync::atomic::{AtomicU64, Ordering},
};

use kairo_core::Config;
use kairo_runtime::{Runtime, RuntimeError};
use sha2::{Digest, Sha256};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);

impl Fixture {
    fn large_input() -> Self {
        let sequence = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("kairo-stream-{}-{sequence}.bin", process::id()));
        let mut file = File::create(&path).expect("fixture should be created");
        let chunk: Vec<u8> = (0..=255).collect();
        for _ in 0..8192 {
            file.write_all(&chunk).expect("fixture should be written");
        }
        Self(path)
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

#[tokio::test]
async fn matches_the_materialized_baseline() {
    let runtime = Runtime::new(Config::default()).expect("runtime should initialize");
    let workflow = runtime
        .load_workflow(repository_path("demos/stream/workflow.yaml"))
        .expect("stream workflow should load");

    let direct = runtime
        .run_stream_workflow(&workflow, None, false)
        .await
        .expect("direct stream should run");
    let materialized = runtime
        .run_stream_workflow(&workflow, None, true)
        .await
        .expect("materialized stream should run");

    assert_eq!(direct.bytes, materialized.bytes);
    assert_eq!(direct.checksum, materialized.checksum);
    assert_eq!(direct.metrics.materialized_bytes, 0);
    assert_eq!(materialized.metrics.materialized_bytes, materialized.bytes);
}

#[tokio::test]
async fn bounds_large_file_batches() {
    let fixture = Fixture::large_input();
    let runtime = Runtime::new(Config {
        max_stream_input_bytes: 3 * 1024 * 1024,
        stream_chunk_bytes: 1024,
        execution_fuel: 100_000_000,
        ..Config::default()
    })
    .expect("runtime should initialize");
    let workflow = runtime
        .load_workflow(repository_path("demos/stream/workflow.yaml"))
        .expect("stream workflow should load");

    let result = runtime
        .run_stream_workflow(&workflow, Some(fixture.path()), false)
        .await
        .expect("large stream should run");

    assert_eq!(result.bytes, 2 * 1024 * 1024);
    assert_eq!(result.metrics.source_bytes, result.bytes);
    assert_eq!(result.metrics.consumed_bytes, result.bytes);
    assert!(result.metrics.largest_batch_bytes <= 1024);
    assert_eq!(result.metrics.materialized_bytes, 0);
}

#[tokio::test]
async fn rejects_stream_inputs_over_the_configured_limit() {
    let runtime = Runtime::new(Config {
        max_stream_input_bytes: 1,
        ..Config::default()
    })
    .expect("runtime should initialize");
    let workflow = runtime
        .load_workflow(repository_path("demos/stream/workflow.yaml"))
        .expect("stream workflow should load");

    assert!(matches!(
        runtime.run_stream_workflow(&workflow, None, false).await,
        Err(RuntimeError::StreamInputTooLarge { max_bytes: 1, .. })
    ));
}

#[tokio::test]
async fn preserves_fuel_errors_from_stream_steps() {
    let runtime = Runtime::new(Config {
        execution_fuel: 1,
        ..Config::default()
    })
    .expect("runtime should initialize");
    let workflow = runtime
        .load_workflow(repository_path("demos/stream/workflow.yaml"))
        .expect("stream workflow should load");

    let error = runtime
        .run_stream_workflow(&workflow, None, false)
        .await
        .expect_err("limited stream should fail");
    assert!(
        matches!(
            error,
        RuntimeError::StreamStep { ref step, ref source }
                if step == "transform" && matches!(**source, RuntimeError::FuelExhausted { .. })
        ),
        "{error:?}"
    );
}

#[tokio::test]
async fn cleans_up_when_a_consumer_drops_its_stream() {
    let workflow = kairo_core::Workflow::parse(
        r#"
workflow: drop-input
mode: stream
input: demos/stream/input.bin
steps:
  - { name: transform, component: demos/stream/transform.wat }
  - { name: consume, component: components/runtime/stream-drop.wat }
edges:
  - { from: transform, to: consume }
"#,
        &repository_path(""),
        256,
    )
    .expect("workflow should parse");
    let runtime = Runtime::new(Config::default()).expect("runtime should initialize");

    let result = runtime
        .run_stream_workflow(&workflow, None, false)
        .await
        .expect("dropped stream should complete");

    assert_eq!(result.bytes, 0);
    assert_eq!(result.metrics.source_bytes, 0);
    assert_eq!(result.metrics.consumed_bytes, 0);
}

#[test]
fn rejects_components_without_the_stream_transform_interface() {
    let workflow = kairo_core::Workflow::parse(
        r#"
workflow: invalid
mode: stream
input: demos/stream/input.bin
steps:
  - { name: transform, component: components/probe/component.wat }
  - { name: consume, component: demos/stream/consume.wat }
edges:
  - { from: transform, to: consume }
"#,
        &repository_path(""),
        256,
    )
    .expect("workflow should parse");
    let runtime = Runtime::new(Config::default()).expect("runtime should initialize");

    assert!(matches!(
        runtime.validate_stream_workflow(&workflow),
        Err(RuntimeError::IncompatibleStreamComponent {
            role: "transform",
            ..
        })
    ));
}

#[test]
fn rejects_zero_stream_chunks() {
    assert!(matches!(
        Runtime::new(Config {
            stream_chunk_bytes: 0,
            ..Config::default()
        }),
        Err(RuntimeError::InvalidStreamChunkSize)
    ));
}

#[tokio::test]
async fn streams_a_file_directly_between_components() {
    let input = repository_path("demos/stream/input.bin");
    let expected = fs::read(&input).expect("demo input should load");
    let runtime = Runtime::new(Config::default()).expect("runtime should initialize");
    let workflow = runtime
        .load_workflow(repository_path("demos/stream/workflow.yaml"))
        .expect("stream workflow should load");

    let result = runtime
        .run_stream_workflow(&workflow, None, false)
        .await
        .expect("stream workflow should run");

    assert_eq!(result.bytes, expected.len() as u64);
    assert_eq!(
        result.checksum,
        expected.iter().fold(0_u32, |checksum, byte| checksum
            .wrapping_add(u32::from(*byte)))
    );
    assert_eq!(result.metrics.source_bytes, result.bytes);
    assert_eq!(result.metrics.consumed_bytes, result.bytes);
    assert!(result.metrics.largest_batch_bytes <= Config::default().stream_chunk_bytes);
    assert_eq!(
        result.input_hash,
        format!("sha256:{:x}", Sha256::digest(&expected))
    );
}

#[tokio::test]
async fn distinguishes_missing_and_non_file_inputs() {
    let runtime = Runtime::new(Config::default()).expect("runtime should initialize");
    let workflow = runtime
        .load_workflow(repository_path("demos/stream/workflow.yaml"))
        .expect("stream workflow should load");
    let missing = std::env::temp_dir().join("kairo-runtime-missing-input");

    assert!(matches!(
        runtime
            .run_stream_workflow(&workflow, Some(&missing), false)
            .await,
        Err(RuntimeError::StreamInputNotFound { .. })
    ));
    assert!(matches!(
        runtime
            .run_stream_workflow(&workflow, Some(&std::env::temp_dir()), false)
            .await,
        Err(RuntimeError::StreamInputNotRegular { .. })
    ));
}
