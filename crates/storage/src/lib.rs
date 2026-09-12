use std::{
    fs,
    io::{Read, Write},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use object_store::{
    MultipartUpload, ObjectStore, ObjectStoreExt, PutPayload, aws::AmazonS3Builder, path::Path,
};
use sha2::{Digest, Sha256};
use thiserror::Error;

const ARTIFACT_VERSION: u8 = 1;
const ARTIFACT_PREFIX: &[u8] = b"kairo-artifact";
const MULTIPART_PART_BYTES: usize = 5 * 1024 * 1024;
static NEXT_STAGING_OBJECT: AtomicU64 = AtomicU64::new(0);
pub const LOCAL_ENDPOINT: &str = "http://127.0.0.1:9000";
pub const LOCAL_BUCKET: &str = "kairo";

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct StorageConfig {
    pub endpoint: String,
    pub bucket: String,
    pub local: bool,
}

impl StorageConfig {
    pub fn from_env() -> Result<Self, StorageError> {
        Ok(Self {
            endpoint: environment("KAIRO_ARTIFACT_ENDPOINT")?,
            bucket: environment("KAIRO_ARTIFACT_BUCKET")?,
            local: false,
        })
    }

    pub fn local() -> Self {
        Self {
            endpoint: ".kairo/artifacts".to_owned(),
            bucket: String::new(),
            local: true,
        }
    }

    pub fn minio() -> Self {
        Self {
            endpoint: LOCAL_ENDPOINT.to_owned(),
            bucket: LOCAL_BUCKET.to_owned(),
            local: false,
        }
    }
}

#[derive(Clone)]
pub struct ArtifactStore {
    store: Arc<dyn ObjectStore>,
    backend: ArtifactBackend,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArtifactBackend {
    Local,
    R2,
    Minio,
    External,
    Memory,
}

impl ArtifactBackend {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::R2 => "r2",
            Self::Minio => "minio",
            Self::External => "external",
            Self::Memory => "memory",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Artifact {
    pub hash: String,
    pub value: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ByteArtifact {
    pub hash: String,
    pub bytes: u64,
    pub reference: String,
}

pub struct ByteArtifactWriter {
    store: Arc<dyn ObjectStore>,
    temporary: PathBuf,
    file: fs::File,
    hasher: Sha256,
    bytes: u64,
    maximum: u64,
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
        let backend = if config.endpoint.ends_with(".r2.cloudflarestorage.com") {
            ArtifactBackend::R2
        } else if config.endpoint == LOCAL_ENDPOINT {
            ArtifactBackend::Minio
        } else {
            ArtifactBackend::External
        };
        let (access_key, secret_key) = credentials(&config)?;
        let builder = AmazonS3Builder::new()
            .with_endpoint(&config.endpoint)
            .with_bucket_name(config.bucket)
            .with_access_key_id(access_key)
            .with_secret_access_key(secret_key)
            .with_allow_http(config.endpoint.starts_with("http://"))
            .with_virtual_hosted_style_request(false);
        let store = builder
            .build()
            .map_err(|source| StorageError::Configure { source })?;
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
        let temporary = temporary_path();
        let file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|source| StorageError::CreateTemporary { source })?;
        Ok(ByteArtifactWriter {
            store: Arc::clone(&self.store),
            temporary,
            file,
            hasher: Sha256::new(),
            bytes: 0,
            maximum,
        })
    }

    pub async fn get_bytes(&self, hash: &str) -> Result<Vec<u8>, StorageError> {
        self.read_verified(hash, &byte_path(hash)).await
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

impl ByteArtifactWriter {
    pub fn write(&mut self, bytes: &[u8]) -> Result<(), StorageError> {
        let next = self
            .bytes
            .checked_add(u64::try_from(bytes.len()).unwrap_or(u64::MAX))
            .ok_or(StorageError::OutputTooLarge {
                maximum: self.maximum,
            })?;
        if next > self.maximum {
            return Err(StorageError::OutputTooLarge {
                maximum: self.maximum,
            });
        }
        self.bytes = next;
        self.hasher.update(bytes);
        self.file
            .write_all(bytes)
            .map_err(|source| StorageError::WriteTemporary { source })?;
        Ok(())
    }

    pub async fn finish(mut self) -> Result<ByteArtifact, StorageError> {
        self.file
            .flush()
            .and_then(|()| self.file.sync_all())
            .map_err(|source| StorageError::WriteTemporary { source })?;
        let hash = format!("sha256:{:x}", self.hasher.finalize());
        let staging = staging_path();
        let mut upload = self
            .store
            .put_multipart(&staging)
            .await
            .map_err(|source| StorageError::BeginUpload { source })?;
        let mut input = fs::File::open(&self.temporary)
            .map_err(|source| StorageError::ReadTemporary { source })?;
        let mut buffer = vec![0; MULTIPART_PART_BYTES];
        let mut wrote = false;
        loop {
            let count = input
                .read(&mut buffer)
                .map_err(|source| StorageError::ReadTemporary { source })?;
            if count == 0 {
                break;
            }
            wrote = true;
            upload
                .put_part(PutPayload::from(buffer[..count].to_vec()))
                .await
                .map_err(|source| StorageError::WriteUpload { source })?;
        }
        if !wrote {
            upload
                .put_part(PutPayload::from(Vec::new()))
                .await
                .map_err(|source| StorageError::WriteUpload { source })?;
        }
        upload
            .complete()
            .await
            .map_err(|source| StorageError::CompleteUpload { source })?;
        let artifact_path = byte_path(&hash);
        let published = self
            .store
            .copy_if_not_exists(&staging, &artifact_path)
            .await;
        if let Err(source) = published
            && !matches!(source, object_store::Error::AlreadyExists { .. })
        {
            return Err(StorageError::Publish { hash, source });
        }
        let _ = self.store.delete(&staging).await;
        let _ = fs::remove_file(&self.temporary);
        Ok(ByteArtifact {
            reference: artifact_path.to_string(),
            hash,
            bytes: self.bytes,
        })
    }

    pub fn abort(self) {
        let _ = fs::remove_file(self.temporary);
    }
}

fn environment(name: &'static str) -> Result<String, StorageError> {
    std::env::var(name).map_err(|_| StorageError::MissingConfiguration { name })
}

fn credentials(config: &StorageConfig) -> Result<(String, String), StorageError> {
    if config.endpoint.ends_with(".r2.cloudflarestorage.com") {
        return credential_pair(
            "KAIRO_R2_ACCESS_KEY_ID",
            "KAIRO_R2_SECRET_ACCESS_KEY",
            StorageError::MissingR2Credentials,
        );
    }
    if config.endpoint == LOCAL_ENDPOINT {
        return credential_pair(
            "KAIRO_MINIO_ACCESS_KEY_ID",
            "KAIRO_MINIO_SECRET_ACCESS_KEY",
            StorageError::MissingMinioCredentials,
        );
    }
    credential_pair(
        "KAIRO_ARTIFACT_ACCESS_KEY_ID",
        "KAIRO_ARTIFACT_SECRET_ACCESS_KEY",
        StorageError::MissingArtifactCredentials,
    )
}

fn credential_pair(
    access_key_name: &'static str,
    secret_key_name: &'static str,
    missing: StorageError,
) -> Result<(String, String), StorageError> {
    match (
        std::env::var(access_key_name),
        std::env::var(secret_key_name),
    ) {
        (Ok(access_key), Ok(secret_key)) => Ok((access_key, secret_key)),
        _ => Err(missing),
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

fn byte_path(hash: &str) -> Path {
    Path::from(format!("outputs/{hash}"))
}

fn staging_path() -> Path {
    let sequence = NEXT_STAGING_OBJECT.fetch_add(1, Ordering::Relaxed);
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    Path::from(format!("staging/{timestamp}-{sequence}"))
}

fn temporary_path() -> PathBuf {
    let sequence = NEXT_STAGING_OBJECT.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("kairo-output-{}-{sequence}", std::process::id()))
}
