#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::path::{Path, PathBuf};

use kairo_core::{Config, Workflow};
use kairo_runtime::{Runtime, StreamGroupInput, StreamGroupOutcome, StreamRelay};
use kairo_storage::ArtifactStore;

fn repository_path(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(path)
}

fn required_boundary_workflow() -> Workflow {
    let transform = repository_path("demos/stream/transform.wat");
    let consume = repository_path("demos/stream/consume.wat");
    let source = format!(
        "workflow: byte-checksum\nmode: stream\ninput: {input}\nsteps:\n  - name: transform\n    component: {transform}\n  - name: consume\n    component: {consume}\nedges:\n  - from: transform\n    to: consume\n    durability: required\n",
        input = repository_path("demos/stream/input.bin").display(),
        transform = transform.display(),
        consume = consume.display(),
    );
    Workflow::parse(&source, Path::new("."), 8).expect("workflow should parse")
}

#[tokio::test]
async fn a_required_boundary_materializes_a_real_artifact_and_the_next_group_reuses_it() {
    let runtime = Runtime::new(Config::default()).expect("runtime should initialize");
    let workflow = required_boundary_workflow();
    let artifacts = ArtifactStore::memory();

    let baseline = runtime
        .run_stream_workflow(&workflow, None, false)
        .await
        .expect("single-shot baseline should run");

    let first = runtime
        .run_stream_cell_group(
            &workflow,
            Some(&artifacts),
            0,
            StreamGroupInput::File(&repository_path("demos/stream/input.bin")),
            Some(0),
        )
        .await
        .expect("first group should run");
    let StreamGroupOutcome::Yielded {
        next_index,
        artifact_hash,
        ..
    } = first
    else {
        unreachable!("a non-final group must yield, not complete the workflow");
    };
    assert_eq!(next_index, 1);

    let materialized = artifacts
        .get_bytes(&artifact_hash)
        .await
        .expect("the materialized checkpoint should be fetchable from durable storage");
    assert!(
        !materialized.is_empty(),
        "a real transform output should never materialize to zero bytes for this fixture"
    );

    let second = runtime
        .run_stream_cell_group(
            &workflow,
            Some(&artifacts),
            1,
            StreamGroupInput::Bytes(materialized),
            None,
        )
        .await
        .expect("second group should run");
    let StreamGroupOutcome::Completed(result) = second else {
        unreachable!("the final group must complete, not yield");
    };

    assert_eq!(result.bytes, baseline.bytes);
    assert_eq!(result.checksum, baseline.checksum);
}

#[tokio::test]
async fn a_component_stream_crosses_a_bounded_live_relay_without_materialization() {
    let runtime = Runtime::new(Config::default()).expect("runtime should initialize");
    let workflow = required_boundary_workflow();
    let baseline = runtime
        .run_stream_workflow(&workflow, None, false)
        .await
        .expect("single-shot baseline should run");
    let (sink, source) = StreamRelay::bounded(256 * 1024);
    let (_, received) = tokio::try_join!(
        async {
            runtime
                .relay_stream_prefix(
                    &workflow,
                    &repository_path("demos/stream/input.bin"),
                    1,
                    sink,
                )
                .await
        },
        async { runtime.consume_relay_suffix(&workflow, 1, source).await },
    )
    .expect("live relay should preserve the component stream");
    assert_eq!(received.bytes, baseline.bytes);
    assert_eq!(received.checksum, baseline.checksum);
}

#[tokio::test]
async fn demo_consumer_handles_an_initially_blocked_stream_read() {
    let runtime = Runtime::new(Config::default()).expect("runtime should initialize");
    let workflow = required_boundary_workflow();
    let (sink, source) = StreamRelay::bounded(256 * 1024);
    let (result, ()) = tokio::join!(
        async { runtime.consume_relay_suffix(&workflow, 1, source).await },
        async {
            sink.wait_for_producer_polls(1)
                .await
                .expect("consumer should begin a stream read");
            sink.send(vec![1_u8])
                .await
                .expect("relay should accept a byte");
            sink.close();
        },
    );
    let result = result.expect("consumer should wait for the readable stream event");
    assert_eq!(result.bytes, 1);
    assert_eq!(result.checksum, 1);
    assert!(sink.metrics().producer_polls >= 3);
    assert!(sink.metrics().producer_pending >= 1);
}

#[tokio::test]
async fn demo_consumer_handles_immediately_available_data_and_clean_eof() {
    let runtime = Runtime::new(Config::default()).expect("runtime should initialize");
    let workflow = required_boundary_workflow();
    let (sink, source) = StreamRelay::bounded(256 * 1024);
    sink.send(vec![1_u8, 2, 3])
        .await
        .expect("relay should accept bytes");
    sink.close();
    let result = runtime
        .consume_relay_suffix(&workflow, 1, source)
        .await
        .expect("consumer should accept a ready stream and EOF");
    assert_eq!(result.bytes, 3);
    assert_eq!(result.checksum, 6);
}

#[tokio::test]
async fn demo_consumer_handles_multiple_blocked_read_cycles() {
    let runtime = Runtime::new(Config::default()).expect("runtime should initialize");
    let workflow = required_boundary_workflow();
    let (sink, source) = StreamRelay::bounded(256 * 1024);
    let (result, ()) = tokio::join!(
        async { runtime.consume_relay_suffix(&workflow, 1, source).await },
        async {
            sink.wait_for_producer_polls(1)
                .await
                .expect("first read should block");
            sink.send(vec![4_u8])
                .await
                .expect("first byte should be accepted");
            sink.wait_for_producer_polls(3)
                .await
                .expect("second read should block");
            sink.send(vec![5_u8])
                .await
                .expect("second byte should be accepted");
            sink.wait_for_producer_polls(5)
                .await
                .expect("third read should block");
            sink.close();
        },
    );
    let result = result.expect("consumer should resume after every stream event");
    assert_eq!(result.bytes, 2);
    assert_eq!(result.checksum, 9);
    assert!(sink.metrics().producer_pending >= 3);
}

#[tokio::test]
async fn demo_consumer_reports_relay_failure() {
    let runtime = Runtime::new(Config::default()).expect("runtime should initialize");
    let workflow = required_boundary_workflow();
    let (sink, source) = StreamRelay::bounded(256 * 1024);
    let (result, ()) = tokio::join!(
        async { runtime.consume_relay_suffix(&workflow, 1, source).await },
        async {
            sink.wait_for_producer_polls(1)
                .await
                .expect("consumer should begin a stream read");
            sink.fail("transport interrupted");
        },
    );
    assert!(result.is_err(), "a transport failure must not become EOF");
}
