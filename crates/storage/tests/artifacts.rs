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
