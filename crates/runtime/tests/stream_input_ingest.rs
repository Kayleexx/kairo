#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::{fs, path::PathBuf, process};

use kairo_core::Config;
use kairo_runtime::Runtime;
use kairo_storage::ArtifactStore;

fn temp_file(name: &str, bytes: &[u8]) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "kairo-ingest-{name}-{}-{}",
        process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::write(&path, bytes).expect("fixture input should be written");
    path
}

#[tokio::test]
async fn ingests_a_file_and_reads_it_back_byte_identical() {
    let runtime = Runtime::new(Config::default()).expect("runtime should initialize");
    let artifacts = ArtifactStore::memory();
    let bytes = b"the quick brown fox jumps over the lazy dog".repeat(1000);
    let path = temp_file("roundtrip", &bytes);

    let artifact = runtime
        .ingest_stream_input(&path, &artifacts, 10 * 1024 * 1024)
        .await
        .expect("ingest should succeed");
    assert_eq!(artifact.bytes, bytes.len() as u64);

    let read_back = artifacts
        .get_bytes(&artifact.hash)
        .await
        .expect("artifact should read back");
    assert_eq!(read_back, bytes);

    let _ = fs::remove_file(&path);
}

#[tokio::test]
async fn ingests_a_zero_byte_file() {
    let runtime = Runtime::new(Config::default()).expect("runtime should initialize");
    let artifacts = ArtifactStore::memory();
    let path = temp_file("empty", b"");

    let artifact = runtime
        .ingest_stream_input(&path, &artifacts, 1024)
        .await
        .expect("ingest should succeed");
    assert_eq!(artifact.bytes, 0);
    let read_back = artifacts
        .get_bytes(&artifact.hash)
        .await
        .expect("artifact should read back");
    assert!(read_back.is_empty());

    let _ = fs::remove_file(&path);
}

#[tokio::test]
async fn rejects_input_over_the_bound_without_materializing_it() {
    let runtime = Runtime::new(Config::default()).expect("runtime should initialize");
    let artifacts = ArtifactStore::memory();
    let path = temp_file("oversized", &vec![0_u8; 4096]);

    let error = runtime
        .ingest_stream_input(&path, &artifacts, 1024)
        .await
        .expect_err("oversized input should be rejected");
    assert!(matches!(
        error,
        kairo_runtime::RuntimeError::Artifact {
            source: kairo_storage::StorageError::OutputTooLarge { maximum: 1024 }
        }
    ));

    let _ = fs::remove_file(&path);
}

#[tokio::test]
async fn rejects_a_missing_local_file() {
    let runtime = Runtime::new(Config::default()).expect("runtime should initialize");
    let artifacts = ArtifactStore::memory();
    let path = std::env::temp_dir().join(format!("kairo-ingest-missing-{}", process::id()));
    let _ = fs::remove_file(&path);

    let error = runtime.ingest_stream_input(&path, &artifacts, 1024).await;
    assert!(error.is_err());
}

#[tokio::test]
async fn deduplicates_identical_content() {
    let runtime = Runtime::new(Config::default()).expect("runtime should initialize");
    let artifacts = ArtifactStore::memory();
    let path_a = temp_file("dedup-a", b"same content");
    let path_b = temp_file("dedup-b", b"same content");

    let first = runtime
        .ingest_stream_input(&path_a, &artifacts, 1024)
        .await
        .expect("first ingest should succeed");
    let second = runtime
        .ingest_stream_input(&path_b, &artifacts, 1024)
        .await
        .expect("second ingest should succeed");
    assert_eq!(first.hash, second.hash);

    let _ = fs::remove_file(&path_a);
    let _ = fs::remove_file(&path_b);
}
