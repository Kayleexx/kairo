use std::sync::Arc;

use object_store::{
    ObjectStore,
    aws::{AmazonS3Builder, S3CopyIfNotExists},
};

use crate::StorageError;

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

pub(super) fn remote_store(
    config: StorageConfig,
) -> Result<(Arc<dyn ObjectStore>, ArtifactBackend), StorageError> {
    let backend = if config.endpoint.ends_with(".r2.cloudflarestorage.com") {
        ArtifactBackend::R2
    } else if config.endpoint == LOCAL_ENDPOINT {
        ArtifactBackend::Minio
    } else {
        ArtifactBackend::External
    };
    let (access_key, secret_key) = credentials(&config)?;
    let mut builder = AmazonS3Builder::new()
        .with_endpoint(&config.endpoint)
        .with_bucket_name(config.bucket)
        .with_access_key_id(access_key)
        .with_secret_access_key(secret_key)
        .with_allow_http(config.endpoint.starts_with("http://"))
        .with_virtual_hosted_style_request(false);
    if matches!(backend, ArtifactBackend::Minio) {
        // MinIO doesn't support AWS's native conditional-copy semantics, but does honor a plain
        // `If-None-Match: *` header the same way -- without this, publishing a content-addressed
        // artifact (`copy_if_not_exists`) fails outright against MinIO.
        builder = builder.with_copy_if_not_exists(S3CopyIfNotExists::Header(
            "If-None-Match".to_owned(),
            "*".to_owned(),
        ));
    }
    let store = builder
        .build()
        .map_err(|source| StorageError::Configure { source })?;
    Ok((Arc::new(store), backend))
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
