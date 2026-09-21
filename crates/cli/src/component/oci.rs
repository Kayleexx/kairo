use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use docker_credential::DockerCredential;
use kairo_core::{Config, catalog::ComponentSource};
use oci_client::{
    Client, Reference,
    client::{ClientConfig, ClientProtocol},
    manifest::WASM_LAYER_MEDIA_TYPE,
    secrets::RegistryAuth,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use thiserror::Error;

const COMPONENT_MEDIA_TYPES: [&str; 3] = [
    WASM_LAYER_MEDIA_TYPE,
    "application/wasm",
    "application/vnd.w3c.wasm.component.v1+wasm",
];
static TEMPORARY_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub(super) struct ResolvedComponent {
    pub(super) path: PathBuf,
    pub(super) source: ComponentSource,
    pub(super) version: Option<String>,
    pub(super) digest: String,
}

#[derive(Debug, Error)]
pub(crate) enum OciError {
    #[error(
        "`{source}` is not a valid OCI Component reference; use `registry/repository:tag` or a local Component path"
    )]
    InvalidReference {
        source: String,
        #[source]
        error: oci_client::ParseError,
    },
    #[error("OCI Component `{reference}` requires a sha256 digest")]
    UnsupportedDigest { reference: String },
    #[error("OCI Component `{reference}` has no supported WebAssembly Component layer")]
    NoComponentLayer { reference: String },
    #[error("OCI Component `{reference}` has more than one WebAssembly Component layer")]
    MultipleComponentLayers { reference: String },
    #[error("OCI Component `{reference}` exceeds the {max_bytes}-byte Component limit")]
    TooLarge { reference: String, max_bytes: usize },
    #[error(
        "OCI registry request for `{reference}` failed; check the reference and run `docker login {registry}` if it is private"
    )]
    Registry {
        reference: String,
        registry: String,
        #[source]
        source: oci_client::errors::OciDistributionError,
    },
    #[error(
        "OCI Component `{reference}` returned content with digest `{actual}`, expected `{expected}`"
    )]
    DigestMismatch {
        reference: String,
        expected: String,
        actual: String,
    },
    #[error("failed to create OCI Component cache `{path}`")]
    CreateCache {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to write OCI Component cache `{path}`")]
    WriteCache {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to read OCI Component cache `{path}`")]
    ReadCache {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to serialize OCI Component cache metadata")]
    SerializeCache(#[source] toml::ser::Error),
}

#[derive(Deserialize, Serialize)]
struct CacheMetadata {
    schema: u32,
    manifest_digest: String,
    layer_digest: String,
}

pub(super) async fn resolve(source: &str, config: Config) -> Result<ResolvedComponent, OciError> {
    let reference = source
        .parse::<Reference>()
        .map_err(|error| OciError::InvalidReference {
            source: source.to_owned(),
            error,
        })?;
    if let Some(digest) = reference.digest()
        && let Some(component) = load_cached(source, digest, None, reference.tag())?
    {
        return Ok(component);
    }

    let registry = reference.resolve_registry().to_owned();
    let client = Client::new(client_config(&registry));
    let auth = credentials(&registry);
    let (manifest, manifest_digest) = client
        .pull_image_manifest(&reference, &auth)
        .await
        .map_err(|source| OciError::Registry {
            reference: source_text(&reference),
            registry: registry.clone(),
            source,
        })?;
    let layer = manifest
        .layers
        .iter()
        .filter(|layer| COMPONENT_MEDIA_TYPES.contains(&layer.media_type.as_str()))
        .collect::<Vec<_>>();
    let layer = match layer.as_slice() {
        [] => {
            return Err(OciError::NoComponentLayer {
                reference: source_text(&reference),
            });
        }
        [layer] => *layer,
        _ => {
            return Err(OciError::MultipleComponentLayers {
                reference: source_text(&reference),
            });
        }
    };
    let size = usize::try_from(layer.size).ok();
    if size.is_none_or(|size| size > config.max_component_bytes) {
        return Err(OciError::TooLarge {
            reference: source_text(&reference),
            max_bytes: config.max_component_bytes,
        });
    }
    if let Some(component) = load_cached(
        source,
        &manifest_digest,
        Some(&layer.digest),
        reference.tag(),
    )? {
        return Ok(component);
    }
    let image = client
        .pull(&reference, &auth, COMPONENT_MEDIA_TYPES.to_vec())
        .await
        .map_err(|source| OciError::Registry {
            reference: source_text(&reference),
            registry,
            source,
        })?;
    let bytes = image
        .layers
        .into_iter()
        .find(|layer| COMPONENT_MEDIA_TYPES.contains(&layer.media_type.as_str()))
        .map(|layer| layer.data)
        .ok_or_else(|| OciError::NoComponentLayer {
            reference: source_text(&reference),
        })?;
    if bytes.len() > config.max_component_bytes {
        return Err(OciError::TooLarge {
            reference: source_text(&reference),
            max_bytes: config.max_component_bytes,
        });
    }
    let actual = digest(&bytes);
    if actual != layer.digest {
        return Err(OciError::DigestMismatch {
            reference: source_text(&reference),
            expected: layer.digest.clone(),
            actual,
        });
    }
    store_cache(
        &manifest_digest,
        &layer.digest,
        &bytes,
        source,
        reference.tag(),
    )
}

fn client_config(registry: &str) -> ClientConfig {
    let mut config = ClientConfig::default();
    if localhost(registry) {
        config.protocol = ClientProtocol::HttpsExcept(vec![registry.to_owned()]);
    }
    config
}

fn localhost(registry: &str) -> bool {
    registry == "localhost"
        || registry.starts_with("localhost:")
        || registry.starts_with("127.")
        || registry.starts_with("[::1]")
}

fn credentials(registry: &str) -> RegistryAuth {
    let credential = docker_credential::get_credential(registry)
        .or_else(|_| docker_credential::get_podman_credential(registry));
    match credential {
        Ok(DockerCredential::IdentityToken(token)) => RegistryAuth::Bearer(token),
        Ok(DockerCredential::UsernamePassword(username, password)) => {
            RegistryAuth::Basic(username, password)
        }
        Err(_) => RegistryAuth::Anonymous,
    }
}

fn load_cached(
    reference: &str,
    manifest_digest: &str,
    expected_layer: Option<&str>,
    version: Option<&str>,
) -> Result<Option<ResolvedComponent>, OciError> {
    let Some(directory) = cache_directory(manifest_digest) else {
        return Err(OciError::UnsupportedDigest {
            reference: reference.to_owned(),
        });
    };
    let metadata_path = directory.join("component.toml");
    let component_path = directory.join("component.wasm");
    if !metadata_path.is_file() || !component_path.is_file() {
        return Ok(None);
    }
    let metadata_source =
        fs::read_to_string(&metadata_path).map_err(|source| OciError::ReadCache {
            path: metadata_path.clone(),
            source,
        })?;
    let Ok(metadata) = toml::from_str::<CacheMetadata>(&metadata_source) else {
        return Ok(None);
    };
    if metadata.schema != 1
        || metadata.manifest_digest != manifest_digest
        || expected_layer.is_some_and(|layer| layer != metadata.layer_digest)
    {
        return Ok(None);
    }
    let bytes = fs::read(&component_path).map_err(|source| OciError::ReadCache {
        path: component_path.clone(),
        source,
    })?;
    if digest(&bytes) != metadata.layer_digest {
        return Ok(None);
    }
    Ok(Some(ResolvedComponent {
        path: component_path,
        source: component_source(reference, manifest_digest),
        version: version.map(str::to_owned),
        digest: manifest_digest.to_owned(),
    }))
}

fn store_cache(
    manifest_digest: &str,
    layer_digest: &str,
    bytes: &[u8],
    reference: &str,
    version: Option<&str>,
) -> Result<ResolvedComponent, OciError> {
    let directory =
        cache_directory(manifest_digest).ok_or_else(|| OciError::UnsupportedDigest {
            reference: manifest_digest.to_owned(),
        })?;
    fs::create_dir_all(&directory).map_err(|source| OciError::CreateCache {
        path: directory.clone(),
        source,
    })?;
    let component_path = directory.join("component.wasm");
    write_atomic(&component_path, bytes)?;
    let metadata = toml::to_string_pretty(&CacheMetadata {
        schema: 1,
        manifest_digest: manifest_digest.to_owned(),
        layer_digest: layer_digest.to_owned(),
    })
    .map_err(OciError::SerializeCache)?;
    write_atomic(&directory.join("component.toml"), metadata.as_bytes())?;
    Ok(ResolvedComponent {
        path: component_path,
        source: component_source(reference, manifest_digest),
        version: version.map(str::to_owned),
        digest: manifest_digest.to_owned(),
    })
}

fn write_atomic(path: &Path, contents: &[u8]) -> Result<(), OciError> {
    let suffix = TEMPORARY_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temporary = path.with_extension(format!("tmp-{}-{suffix}", std::process::id()));
    fs::write(&temporary, contents).map_err(|source| OciError::WriteCache {
        path: temporary.clone(),
        source,
    })?;
    fs::rename(&temporary, path).map_err(|source| OciError::WriteCache {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(())
}

fn cache_directory(manifest_digest: &str) -> Option<PathBuf> {
    manifest_digest
        .strip_prefix("sha256:")
        .filter(|value| value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .map(|digest| Path::new(".kairo/cache/oci/sha256").join(digest))
}

fn digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn component_source(reference: &str, digest: &str) -> ComponentSource {
    ComponentSource {
        kind: "oci".to_owned(),
        reference: reference.to_owned(),
        resolved_digest: Some(digest.to_owned()),
    }
}

fn source_text(reference: &Reference) -> String {
    reference.whole()
}
