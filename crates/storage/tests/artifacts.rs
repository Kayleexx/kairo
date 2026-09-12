#![allow(clippy::expect_used)]

use kairo_storage::ArtifactStore;

#[tokio::test]
async fn stores_content_addressed_values() {
    let store = ArtifactStore::memory();
    let first = store.put(42).await.expect("artifact should store");
    let second = store.put(42).await.expect("artifact should store");
    let restored = store.get(&first.hash).await.expect("artifact should load");

    assert_eq!(first, second);
    assert_eq!(restored.value, 42);
}

#[tokio::test]
async fn stores_bounded_content_addressed_bytes() {
    let store = ArtifactStore::memory();
    let mut writer = store.begin_bytes(3).await.expect("writer should start");
    writer
        .write(b"abc")
        .expect("limit-sized output should write");
    let artifact = writer.finish().await.expect("artifact should finish");

    assert_eq!(artifact.bytes, 3);
    assert_eq!(
        store
            .get_bytes(&artifact.hash)
            .await
            .expect("artifact should load"),
        b"abc"
    );
}

#[tokio::test]
async fn rejects_byte_output_over_its_limit() {
    let store = ArtifactStore::memory();
    let mut writer = store.begin_bytes(3).await.expect("writer should start");

    assert!(writer.write(b"abcd").is_err());
    writer.abort();
}
