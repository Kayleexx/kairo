use std::{fs, path::PathBuf, sync::Arc};

use object_store::{ObjectStore, ObjectStoreExt, PutPayload, path::Path};
use sha2::{Digest, Sha256};
use thiserror::Error;

mod byte_artifact;
mod configuration;

pub use byte_artifact::ByteArtifactWriter;
pub use configuration::{ArtifactBackend, LOCAL_BUCKET, LOCAL_ENDPOINT, StorageConfig};

const ARTIFACT_VERSION: u8 = 1;
const ARTIFACT_PREFIX: &[u8] = b"kairo-artifact";

#[derive(Clone)]
pub struct ArtifactStore {
    store: Arc<dyn ObjectStore>,
    backend: ArtifactBackend,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Artifact {
    pub hash: String,
    pub value: u32,
    pub bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ByteArtifact {
    pub hash: String,
    pub bytes: u64,
    pub reference: String,
}

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("missing required environment variable `{name}`")]
    MissingConfiguration { name: &'static str },
    #[error("R2 credentials need `KAIRO_R2_ACCESS_KEY_ID` and `KAIRO_R2_SECRET_ACCESS_KEY`")]
    MissingR2Credentials,
    #[error(
        "MinIO credentials need `KAIRO_MINIO_ACCESS_KEY_ID` and `KAIRO_MINIO_SECRET_ACCESS_KEY`"
    )]
    MissingMinioCredentials,
    #[error(
        "storage credentials need `KAIRO_ARTIFACT_ACCESS_KEY_ID` and `KAIRO_ARTIFACT_SECRET_ACCESS_KEY`"
    )]
    MissingArtifactCredentials,
    #[error("invalid artifact storage configuration")]
    Configure {
        #[source]
        source: object_store::Error,
    },
    #[error("failed to create local artifact directory `{path}`")]
    CreateDirectory {
        path: PathBuf,
        #[source]
        source: std::io::Error,
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
    #[error("failed to start artifact upload")]
    BeginUpload {
        #[source]
        source: object_store::Error,
    },
    #[error("failed to create temporary artifact output")]
    CreateTemporary {
        #[source]
        source: std::io::Error,
    },
    #[error("failed to write temporary artifact output")]
    WriteTemporary {
        #[source]
        source: std::io::Error,
    },
    #[error("failed to read temporary artifact output")]
    ReadTemporary {
        #[source]
        source: std::io::Error,
    },
    #[error("artifact output exceeds the {maximum}-byte size limit")]
    OutputTooLarge { maximum: u64 },
    #[error("failed to write artifact upload")]
    WriteUpload {
        #[source]
        source: object_store::Error,
    },
    #[error("failed to finalize artifact upload")]
    CompleteUpload {
        #[source]
        source: object_store::Error,
    },
    #[error("failed to publish artifact `{hash}`")]
    Publish {
        hash: String,
        #[source]
        source: object_store::Error,
    },
}

impl ArtifactStore {
    pub fn from_env() -> Result<Self, StorageError> {
        Self::from_config(StorageConfig::from_env()?)
    }

    pub fn from_config(config: StorageConfig) -> Result<Self, StorageError> {
        if config.local {
            let root = PathBuf::from(config.endpoint);
            fs::create_dir_all(&root).map_err(|source| StorageError::CreateDirectory {
                path: root.clone(),
                source,
            })?;
            let store = object_store::local::LocalFileSystem::new_with_prefix(root)
                .map_err(|source| StorageError::Configure { source })?;
            return Ok(Self {
                store: Arc::new(store),
                backend: ArtifactBackend::Local,
            });
        }
        let (store, backend) = configuration::remote_store(config)?;
        Ok(Self {
            store: Arc::new(store),
            backend,
        })
    }

    pub fn memory() -> Self {
        Self {
            store: Arc::new(object_store::memory::InMemory::new()),
            backend: ArtifactBackend::Memory,
        }
    }

    pub fn backend(&self) -> ArtifactBackend {
        self.backend
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

    pub async fn begin_bytes(&self, maximum: u64) -> Result<ByteArtifactWriter, StorageError> {
        ByteArtifactWriter::new(Arc::clone(&self.store), maximum)
    }

    pub async fn get_bytes(&self, hash: &str) -> Result<Vec<u8>, StorageError> {
        self.read_verified(hash, &byte_artifact::path(hash)).await
    }

    pub async fn check(&self) -> Result<Artifact, StorageError> {
        self.put(0).await
    }

    pub async fn get(&self, hash: &str) -> Result<Artifact, StorageError> {
        let bytes = self.read_verified(hash, &path(hash)).await?;
        let value = decode(&bytes).ok_or_else(|| StorageError::Invalid {
            hash: hash.to_owned(),
        })?;
        Ok(Artifact {
            hash: hash.to_owned(),
            value,
            bytes: bytes.len() as u64,
        })
    }

    async fn read_verified(&self, hash: &str, location: &Path) -> Result<Vec<u8>, StorageError> {
        let result = self.store.get(location).await.map_err(|source| {
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
        Ok(bytes.to_vec())
    }
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
