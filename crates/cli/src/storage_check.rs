use kairo_storage::ArtifactStore;

use super::{SetupError, StorageCheck, storage_backend, storage_config};

pub(crate) async fn check_storage() -> Result<StorageCheck, SetupError> {
    let storage = storage_config()?;
    let backend = storage_backend(&storage);
    let artifact = ArtifactStore::from_config(storage)?.check().await?;
    Ok(StorageCheck {
        backend,
        hash: artifact.hash,
    })
}

pub(crate) async fn check_storage_input(path: &std::path::Path) -> Result<(), SetupError> {
    let original = std::fs::read(path).map_err(|source| SetupError::ReadFile {
        path: path.to_path_buf(),
        source,
    })?;
    let config = kairo_core::Config::default();
    let artifacts = ArtifactStore::from_config(storage_config()?)?;
    let runtime = kairo_runtime::Runtime::new(config)?;
    let artifact = runtime
        .ingest_stream_input(path, &artifacts, config.max_stream_input_bytes)
        .await?;
    let read_back = artifacts.get_bytes(&artifact.hash).await?;
    if read_back != original {
        return Err(SetupError::InputMismatch {
            path: path.to_path_buf(),
        });
    }
    crate::print_valid(format!(
        "input round-trip verified · {} bytes · {}",
        artifact.bytes, artifact.hash
    ));
    Ok(())
}
