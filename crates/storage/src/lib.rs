use std::sync::Arc;

use object_store::{ObjectStore, ObjectStoreExt, PutPayload, aws::AmazonS3Builder, path::Path};
use sha2::{Digest, Sha256};
use thiserror::Error;

const ARTIFACT_VERSION: u8 = 1;
const ARTIFACT_PREFIX: &[u8] = b"kairo-artifact";

#[derive(Clone)]
pub struct ArtifactStore {
    store: Arc<dyn ObjectStore>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Artifact {
    pub hash: String,
    pub value: u32,
}

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("missing required environment variable `{name}`")]
    MissingConfiguration { name: &'static str },
    #[error("invalid MinIO configuration")]
    Configure {
        #[source]
        source: object_store::Error,
    },
    #[error("failed to store artifact `{hash}`")]
    Put {
        hash: String,
        #[source]
        source: object_store::Error,
    },
    #[error("artifact `{hash}` is missing")]
    Missing { hash: String },
    #[error("failed to load artifact `{hash}`")]
    Get {
        hash: String,
        #[source]
        source: object_store::Error,
    },
    #[error("artifact `{hash}` failed integrity verification")]
    Integrity { hash: String },
    #[error("artifact `{hash}` has an unsupported format")]
    Invalid { hash: String },
}

impl ArtifactStore {
    pub fn from_env() -> Result<Self, StorageError> {
        let endpoint = environment("KAIRO_MINIO_ENDPOINT")?;
        let bucket = environment("KAIRO_ARTIFACT_BUCKET")?;
        let allow_http = endpoint.starts_with("http://");
        let store = AmazonS3Builder::from_env()
            .with_endpoint(&endpoint)
            .with_bucket_name(bucket)
            .with_allow_http(allow_http)
            .with_virtual_hosted_style_request(false)
            .build()
            .map_err(|source| StorageError::Configure { source })?;
        Ok(Self {
            store: Arc::new(store),
        })
    }

    pub fn memory() -> Self {
        Self {
            store: Arc::new(object_store::memory::InMemory::new()),
        }
    }

    pub async fn put(&self, value: u32) -> Result<Artifact, StorageError> {
        let bytes = encode(value);
        let hash = artifact_hash(&bytes);
        self.store
            .put(&path(&hash), PutPayload::from(bytes))
            .await
            .map_err(|source| StorageError::Put {
                hash: hash.clone(),
                source,
            })?;
        self.get(&hash).await
    }

    pub async fn get(&self, hash: &str) -> Result<Artifact, StorageError> {
        let result = self.store.get(&path(hash)).await.map_err(|source| {
            if matches!(source, object_store::Error::NotFound { .. }) {
                StorageError::Missing {
                    hash: hash.to_owned(),
                }
            } else {
                StorageError::Get {
                    hash: hash.to_owned(),
                    source,
                }
            }
        })?;
        let bytes = result.bytes().await.map_err(|source| StorageError::Get {
            hash: hash.to_owned(),
            source,
        })?;
        if artifact_hash(&bytes) != hash {
            return Err(StorageError::Integrity {
                hash: hash.to_owned(),
            });
        }
        let value = decode(&bytes).ok_or_else(|| StorageError::Invalid {
            hash: hash.to_owned(),
        })?;
        Ok(Artifact {
            hash: hash.to_owned(),
            value,
        })
    }
}

fn environment(name: &'static str) -> Result<String, StorageError> {
    std::env::var(name).map_err(|_| StorageError::MissingConfiguration { name })
}

fn encode(value: u32) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(ARTIFACT_PREFIX.len() + 5);
    bytes.extend_from_slice(ARTIFACT_PREFIX);
    bytes.push(ARTIFACT_VERSION);
    bytes.extend_from_slice(&value.to_le_bytes());
    bytes
}

fn decode(bytes: &[u8]) -> Option<u32> {
    let expected = ARTIFACT_PREFIX.len() + 5;
    if bytes.len() != expected || !bytes.starts_with(ARTIFACT_PREFIX) {
        return None;
    }
    let version = *bytes.get(ARTIFACT_PREFIX.len())?;
    if version != ARTIFACT_VERSION {
        return None;
    }
    let value = bytes.get(ARTIFACT_PREFIX.len() + 1..)?;
    Some(u32::from_le_bytes(value.try_into().ok()?))
}

fn artifact_hash(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn path(hash: &str) -> Path {
    Path::from(format!("artifacts/{hash}"))
}
