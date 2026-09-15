use std::{env, fs, path::PathBuf};

use kairo_storage::StorageConfig;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub(crate) enum ConfigError {
    #[error("could not determine a Kairo configuration directory")]
    Directory,
    #[error("failed to create configuration directory `{path}`")]
    CreateDirectory {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to read configuration `{path}`")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("configuration `{path}` is invalid")]
    Parse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
    #[error("failed to write configuration `{path}`")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to encode configuration")]
    Encode {
        #[source]
        source: toml::ser::Error,
    },
    #[error("project worker count must be greater than zero")]
    Workers,
}

#[derive(Default, Deserialize, Serialize)]
struct ConfigFile {
    storage: Option<StorageFile>,
    runtime: Option<RuntimeFile>,
}

#[derive(Deserialize, Serialize)]
struct RuntimeFile {
    workers: Option<usize>,
}

#[derive(Deserialize, Serialize)]
struct StorageFile {
    endpoint: String,
    bucket: String,
    #[serde(default)]
    local: bool,
}

pub(crate) fn load_storage() -> Result<Option<StorageConfig>, ConfigError> {
    let path = path()?;
    let source = match fs::read_to_string(&path) {
        Ok(source) => source,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => return Err(ConfigError::Read { path, source }),
    };
    let config: ConfigFile =
        toml::from_str(&source).map_err(|source| ConfigError::Parse { path, source })?;
    Ok(config.storage.map(|storage| StorageConfig {
        endpoint: storage.endpoint,
        bucket: storage.bucket,
        local: storage.local,
    }))
}

const PROJECT_CONFIG_PATH: &str = ".kairo/config.toml";

fn read_project_config() -> Result<ConfigFile, ConfigError> {
    let path = PathBuf::from(PROJECT_CONFIG_PATH);
    match fs::read_to_string(&path) {
        Ok(source) => toml::from_str(&source).map_err(|source| ConfigError::Parse { path, source }),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(ConfigFile::default()),
        Err(source) => Err(ConfigError::Read { path, source }),
    }
}

fn write_project_config(config: &ConfigFile) -> Result<(), ConfigError> {
    let path = PathBuf::from(PROJECT_CONFIG_PATH);
    let parent = path.parent().ok_or(ConfigError::Directory)?;
    fs::create_dir_all(parent).map_err(|source| ConfigError::CreateDirectory {
        path: parent.to_path_buf(),
        source,
    })?;
    let source = toml::to_string_pretty(config).map_err(|source| ConfigError::Encode { source })?;
    fs::write(&path, source).map_err(|source| ConfigError::Write { path, source })
}

pub(crate) fn project_workers() -> Result<Option<usize>, ConfigError> {
    match read_project_config()?
        .runtime
        .and_then(|runtime| runtime.workers)
    {
        Some(0) => Err(ConfigError::Workers),
        workers => Ok(workers),
    }
}

pub(crate) fn save_project_defaults() -> Result<(), ConfigError> {
    if PathBuf::from(PROJECT_CONFIG_PATH).exists() {
        return Ok(());
    }
    let mut config = read_project_config()?;
    config
        .runtime
        .get_or_insert(RuntimeFile { workers: None })
        .workers
        .get_or_insert(2);
    write_project_config(&config)
}

pub(crate) fn load_project_storage() -> Result<Option<StorageConfig>, ConfigError> {
    Ok(read_project_config()?.storage.map(|storage| StorageConfig {
        endpoint: storage.endpoint,
        bucket: storage.bucket,
        local: storage.local,
    }))
}

pub(crate) fn save_project_storage(storage: Option<&StorageConfig>) -> Result<(), ConfigError> {
    let mut config = read_project_config()?;
    config.storage = storage.map(|storage| StorageFile {
        endpoint: storage.endpoint.clone(),
        bucket: storage.bucket.clone(),
        local: storage.local,
    });
    write_project_config(&config)
}

fn path() -> Result<PathBuf, ConfigError> {
    if let Some(path) = env::var_os("KAIRO_CONFIG") {
        return Ok(PathBuf::from(path));
    }
    let directory = env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .ok_or(ConfigError::Directory)?;
    Ok(directory.join("kairo/config.toml"))
}
