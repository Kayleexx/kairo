use std::{
    env, fs,
    io::{self, IsTerminal, Write},
};

use kairo_storage::{ArtifactStore, StorageConfig};
use thiserror::Error;

use crate::config::{self, ConfigError};

#[path = "minio.rs"]
mod minio;

const DEFAULT_BUCKET: &str = "kairo-artifacts";

#[derive(Debug, Error)]
pub(crate) enum SetupError {
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error(transparent)]
    Storage(#[from] kairo_storage::StorageError),
    #[error("`--local`, `--minio`, `--endpoint`, and `--no-storage` cannot be combined")]
    ConflictingOptions,
    #[error("non-interactive setup needs `--local`, `--minio`, `--endpoint`, or `--no-storage`")]
    NonInteractive,
    #[error("storage endpoint is required for an external profile")]
    EndpointRequired,
    #[error("Cloudflare R2 account ID is required")]
    R2Account,
    #[error("storage backend must be `local` or `r2`")]
    Profile,
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
    #[error("local storage credentials are required")]
    LocalCredentials,
    #[error("local storage credentials may contain only letters, numbers, `_`, and `-`")]
    InvalidLocalCredentials,
    #[error("local storage secret must contain at least eight characters")]
    ShortLocalSecret,
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
    #[error("storage environment needs both `KAIRO_MINIO_ENDPOINT` and `KAIRO_ARTIFACT_BUCKET`")]
    PartialEnvironment,
    #[error("no artifact store is configured; run `kairo init`")]
    MissingStorage,
}

pub(crate) struct InitResult {
    pub(crate) path: std::path::PathBuf,
    pub(crate) storage: Option<StorageConfig>,
}

pub(crate) fn report_initialized(result: InitResult) {
    match result.storage {
        Some(storage) if storage.local => {
            println!("local storage ready · {}", result.path.display());
        }
        Some(storage) => {
            let r2 = storage.endpoint.ends_with(".r2.cloudflarestorage.com");
            let provider = if r2 {
                "R2 configured"
            } else {
                "storage configured"
            };
            println!("{provider} · {}", result.path.display());
            println!(
                "endpoint: {} · bucket: {}",
                storage.endpoint, storage.bucket
            );
            if r2 {
                println!("next: add R2 credentials to .env, then run `kairo storage check`");
            }
        }
        None => println!("storage setup skipped · {}", result.path.display()),
    }
}

struct LocalCredentials {
    access_key: String,
    secret_key: String,
}

pub(crate) fn initialize(
    local: bool,
    minio: bool,
    endpoint: Option<String>,
    bucket: Option<String>,
    no_storage: bool,
) -> Result<InitResult, SetupError> {
    let (selected, minio_selected) = select_storage(local, minio, endpoint, bucket, no_storage)?;
    if minio_selected {
        let credentials = local_credentials()?;
        write_environment(&credentials)?;
        minio::ensure_local_storage(&credentials.access_key, &credentials.secret_key)?;
    }
    let path = config::save_storage(selected.as_ref())?;
    Ok(InitResult {
        path,
        storage: selected,
    })
}

pub(crate) fn artifact_store() -> Result<ArtifactStore, SetupError> {
    let storage = environment_storage()?
        .or(config::load_storage()?)
        .ok_or(SetupError::MissingStorage)?;
    ArtifactStore::from_config(storage).map_err(SetupError::Storage)
}

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
    endpoint: Option<String>,
    bucket: Option<String>,
    no_storage: bool,
) -> Result<(Option<StorageConfig>, bool), SetupError> {
    if bucket.is_some() && endpoint.is_none() {
        return Err(SetupError::EndpointRequired);
    }
    if usize::from(local)
        + usize::from(minio)
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
    if !io::stdin().is_terminal() {
        return Err(SetupError::NonInteractive);
    }
    match prompt("storage backend: local or r2", "local")?.as_str() {
        "local" => Ok((Some(StorageConfig::local()), false)),
        "r2" => Ok((Some(r2_storage()?), false)),
        _ => Err(SetupError::Profile),
    }
}

fn r2_storage() -> Result<StorageConfig, SetupError> {
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

fn environment_storage() -> Result<Option<StorageConfig>, SetupError> {
    let endpoint = env::var_os("KAIRO_MINIO_ENDPOINT");
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

fn local_credentials() -> Result<LocalCredentials, SetupError> {
    let access_key = match env::var("AWS_ACCESS_KEY_ID") {
        Ok(value) => value,
        Err(_) if io::stdin().is_terminal() => prompt("local access key", "kairo")?,
        Err(_) => return Err(SetupError::LocalCredentials),
    };
    let secret_key = match env::var("AWS_SECRET_ACCESS_KEY") {
        Ok(value) => value,
        Err(_) if io::stdin().is_terminal() => prompt("local secret", "")?,
        Err(_) => return Err(SetupError::LocalCredentials),
    };
    if access_key.is_empty() || secret_key.is_empty() {
        return Err(SetupError::LocalCredentials);
    }
    if !access_key.chars().all(valid_credential) || !secret_key.chars().all(valid_credential) {
        return Err(SetupError::InvalidLocalCredentials);
    }
    if secret_key.len() < 8 {
        return Err(SetupError::ShortLocalSecret);
    }
    Ok(LocalCredentials {
        access_key,
        secret_key,
    })
}

fn valid_credential(character: char) -> bool {
    character.is_ascii_alphanumeric() || matches!(character, '_' | '-')
}

fn write_environment(credentials: &LocalCredentials) -> Result<(), SetupError> {
    let existing = match fs::read_to_string(".env") {
        Ok(existing) => Some(existing),
        Err(source) if source.kind() == io::ErrorKind::NotFound => None,
        Err(source) => return Err(SetupError::ReadEnv { source }),
    };
    if let Some(existing) = existing {
        let access_key = format!("AWS_ACCESS_KEY_ID={}", credentials.access_key);
        let secret_key = format!("AWS_SECRET_ACCESS_KEY={}", credentials.secret_key);
        if existing.lines().any(|line| line == access_key)
            && existing.lines().any(|line| line == secret_key)
        {
            return Ok(());
        }
        return Err(SetupError::EnvExists);
    }
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(".env")
        .map_err(|source| SetupError::WriteEnv { source })?;
    writeln!(file, "AWS_ACCESS_KEY_ID={}", credentials.access_key)
        .and_then(|_| writeln!(file, "AWS_SECRET_ACCESS_KEY={}", credentials.secret_key))
        .and_then(|_| file.sync_all())
        .map_err(|source| SetupError::WriteEnv { source })?;
    restrict_environment_permissions()?;
    Ok(())
}

#[cfg(unix)]
fn restrict_environment_permissions() -> Result<(), SetupError> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(".env", fs::Permissions::from_mode(0o600))
        .map_err(|source| SetupError::WriteEnv { source })
}

#[cfg(not(unix))]
fn restrict_environment_permissions() -> Result<(), SetupError> {
    Ok(())
}
