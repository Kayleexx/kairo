use std::{
    env, fs,
    io::{self, IsTerminal, Write},
};

use kairo_storage::{ArtifactStore, StorageConfig};
use thiserror::Error;

use crate::config::{self, ConfigError};
use crate::recipe_templates::{self, RecipeTemplateError};
use crate::starter_components;

#[path = "credentials.rs"]
mod credentials;
#[path = "minio.rs"]
mod minio;
#[path = "storage_check.rs"]
mod storage_check;

const DEFAULT_BUCKET: &str = "kairo-artifacts";

#[derive(Debug, Error)]
pub(crate) enum SetupError {
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error(transparent)]
    Storage(#[from] kairo_storage::StorageError),
    #[error("`--local`, `--minio`, `--r2`, `--endpoint`, and `--no-storage` cannot be combined")]
    ConflictingOptions,
    #[error("storage endpoint is required for an external profile")]
    EndpointRequired,
    #[error("Cloudflare R2 account ID is required")]
    R2Account,
    #[error("failed to read setup input")]
    ReadInput {
        #[source]
        source: io::Error,
    },
    #[error("failed to write setup prompt")]
    WritePrompt {
        #[source]
        source: io::Error,
    },
    #[error("MinIO credentials are required")]
    MinioCredentials,
    #[error("failed to generate a MinIO credential")]
    GenerateCredential {
        #[source]
        source: getrandom::Error,
    },
    #[error("MinIO credentials may contain only letters, numbers, `_`, and `-`")]
    InvalidMinioCredentials,
    #[error("MinIO secret must contain at least eight characters")]
    ShortMinioSecret,
    #[error("refusing to overwrite existing `.env`")]
    EnvExists,
    #[error("failed to read `.env`")]
    ReadEnv {
        #[source]
        source: io::Error,
    },
    #[error("failed to write `.env`")]
    WriteEnv {
        #[source]
        source: io::Error,
    },
    #[error("failed to load `.env`")]
    LoadEnv {
        #[source]
        source: dotenvy::Error,
    },
    #[error("failed to run Docker while trying to {operation}")]
    DockerStart {
        operation: &'static str,
        #[source]
        source: io::Error,
    },
    #[error("Docker could not {operation}: {message}")]
    Docker {
        operation: &'static str,
        message: String,
    },
    #[error("storage environment needs both `KAIRO_ARTIFACT_ENDPOINT` and `KAIRO_ARTIFACT_BUCKET`")]
    PartialEnvironment,
    #[error("no artifact store is configured; run `kairo init`")]
    MissingStorage,
    #[error("failed to create local Kairo state")]
    ProjectState {
        #[source]
        source: io::Error,
    },
    #[error(transparent)]
    Recipes(#[from] RecipeTemplateError),
    #[error(transparent)]
    Starters(#[from] crate::starter_components::StarterComponentError),
    #[error(transparent)]
    Ingest(#[from] kairo_runtime::RuntimeError),
    #[error("failed to read `{path}` for storage verification")]
    ReadFile {
        path: std::path::PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("round-trip mismatch: `{path}` read back differently than it was written")]
    InputMismatch { path: std::path::PathBuf },
}

pub(crate) struct InitResult {
    pub(crate) storage: Option<StorageConfig>,
}

pub(crate) struct StorageCheck {
    pub(crate) backend: &'static str,
    pub(crate) hash: String,
}

pub(crate) fn report_initialized(result: InitResult, verified: Option<StorageCheck>) {
    match result.storage {
        Some(storage) if storage.local => {
            println!("initialized kairo\n\nproject   .\nstorage   local\nready     yes");
            if verified.is_none() {
                println!("note      storage will be checked on first durable run");
            }
        }
        Some(storage) => {
            if storage_backend(&storage) == "R2" {
                println!("R2 artifact storage active");
                if !r2_credentials_configured() {
                    println!("next: add R2 credentials to .env, then run `kairo storage check`");
                }
            } else {
                println!("{} artifact storage active", storage_backend(&storage));
            }
        }
        None => println!("artifact storage disabled"),
    }
}

pub(crate) fn initialize(
    local: bool,
    minio: bool,
    r2: bool,
    endpoint: Option<String>,
    bucket: Option<String>,
    no_storage: bool,
) -> Result<InitResult, SetupError> {
    fs::create_dir_all(".kairo").map_err(|source| SetupError::ProjectState { source })?;
    config::save_project_defaults()?;
    let (selected, minio_selected) =
        select_storage(local, minio, r2, endpoint, bucket, no_storage)?;
    if minio_selected {
        let credentials = credentials::minio_credentials()?;
        credentials::write_environment(&credentials)?;
        minio::ensure_local_storage(&credentials.access_key, &credentials.secret_key)?;
    }
    config::save_project_storage(selected.as_ref())?;
    recipe_templates::install()?;
    starter_components::install(kairo_core::Config::default())?;
    Ok(InitResult { storage: selected })
}

pub(crate) fn project_workers() -> Result<Option<usize>, SetupError> {
    config::project_workers().map_err(Into::into)
}

pub(crate) fn artifact_store() -> Result<ArtifactStore, SetupError> {
    let storage = storage_config()?;
    ArtifactStore::from_config(storage).map_err(SetupError::Storage)
}

pub(crate) fn ensure_storage() -> Result<bool, SetupError> {
    match storage_config() {
        Ok(_) => Ok(false),
        Err(SetupError::MissingStorage) => {
            config::save_project_storage(Some(&StorageConfig::local()))?;
            Ok(true)
        }
        Err(error) => Err(error),
    }
}

pub(crate) use storage_check::{check_storage, check_storage_input};

pub(crate) fn load_environment() -> Result<(), SetupError> {
    match dotenvy::from_filename(".env") {
        Ok(_) => Ok(()),
        Err(dotenvy::Error::Io(source)) if source.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(SetupError::LoadEnv { source }),
    }
}

fn select_storage(
    local: bool,
    minio: bool,
    r2: bool,
    endpoint: Option<String>,
    bucket: Option<String>,
    no_storage: bool,
) -> Result<(Option<StorageConfig>, bool), SetupError> {
    if bucket.is_some() && endpoint.is_none() {
        return Err(SetupError::EndpointRequired);
    }
    if usize::from(local)
        + usize::from(minio)
        + usize::from(r2)
        + usize::from(endpoint.is_some())
        + usize::from(no_storage)
        > 1
    {
        return Err(SetupError::ConflictingOptions);
    }
    if no_storage {
        return Ok((None, false));
    }
    if local {
        return Ok((Some(StorageConfig::local()), false));
    }
    if minio {
        return Ok((Some(StorageConfig::minio()), true));
    }
    if r2 {
        return Ok((Some(r2_storage()?), false));
    }
    if let Some(endpoint) = endpoint {
        return Ok((
            Some(StorageConfig {
                endpoint,
                bucket: bucket.unwrap_or_else(|| DEFAULT_BUCKET.to_owned()),
                local: false,
            }),
            false,
        ));
    }
    Ok((Some(StorageConfig::local()), false))
}

fn r2_storage() -> Result<StorageConfig, SetupError> {
    if let Some(storage) = r2_environment_storage() {
        return Ok(storage);
    }
    if !io::stdin().is_terminal() {
        return Err(SetupError::R2Account);
    }
    let account = prompt("Cloudflare R2 account ID", "")?;
    if account.is_empty() {
        return Err(SetupError::R2Account);
    }
    Ok(StorageConfig {
        endpoint: format!("https://{account}.r2.cloudflarestorage.com"),
        bucket: prompt("R2 bucket", DEFAULT_BUCKET)?,
        local: false,
    })
}

fn r2_environment_storage() -> Option<StorageConfig> {
    let endpoint = match env::var_os("KAIRO_ARTIFACT_ENDPOINT") {
        Some(endpoint) => endpoint.to_string_lossy().into_owned(),
        None => r2_endpoint_from_account_id()?,
    };
    let bucket = env::var_os("KAIRO_ARTIFACT_BUCKET")?
        .to_string_lossy()
        .into_owned();
    endpoint
        .ends_with(".r2.cloudflarestorage.com")
        .then_some(StorageConfig {
            endpoint,
            bucket,
            local: false,
        })
}

fn r2_endpoint_from_account_id() -> Option<String> {
    let account_id = env::var_os("KAIRO_R2_ACCOUNT_ID")?;
    Some(format!(
        "https://{}.r2.cloudflarestorage.com",
        account_id.to_string_lossy()
    ))
}

fn environment_storage() -> Result<Option<StorageConfig>, SetupError> {
    let endpoint = env::var_os("KAIRO_ARTIFACT_ENDPOINT");
    let bucket = env::var_os("KAIRO_ARTIFACT_BUCKET");
    match (endpoint, bucket) {
        (None, None) => Ok(None),
        (Some(endpoint), Some(bucket)) => Ok(Some(StorageConfig {
            endpoint: endpoint.to_string_lossy().into_owned(),
            bucket: bucket.to_string_lossy().into_owned(),
            local: false,
        })),
        _ => Err(SetupError::PartialEnvironment),
    }
}

pub(crate) fn storage_config() -> Result<StorageConfig, SetupError> {
    config::load_project_storage()?
        .or(environment_storage()?)
        .or(config::load_storage()?)
        .ok_or(SetupError::MissingStorage)
}

fn storage_backend(storage: &StorageConfig) -> &'static str {
    if storage.local {
        "local"
    } else if storage.endpoint.ends_with(".r2.cloudflarestorage.com") {
        "R2"
    } else if storage.endpoint == kairo_storage::LOCAL_ENDPOINT {
        "MinIO"
    } else {
        "external"
    }
}

fn prompt(label: &str, default: &str) -> Result<String, SetupError> {
    print!("{label} [{default}]: ");
    io::stdout()
        .flush()
        .map_err(|source| SetupError::WritePrompt { source })?;
    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .map_err(|source| SetupError::ReadInput { source })?;
    let input = input.trim();
    Ok(if input.is_empty() { default } else { input }.to_owned())
}

fn r2_credentials_configured() -> bool {
    env::var_os("KAIRO_R2_ACCESS_KEY_ID").is_some()
        && env::var_os("KAIRO_R2_SECRET_ACCESS_KEY").is_some()
}
