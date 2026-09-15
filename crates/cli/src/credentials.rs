use std::{
    env, fs,
    io::{self, IsTerminal, Write},
};

use super::{SetupError, prompt};

pub(super) struct MinioCredentials {
    pub(super) access_key: String,
    pub(super) secret_key: String,
}

pub(super) fn minio_credentials() -> Result<MinioCredentials, SetupError> {
    let access_key = match env::var("KAIRO_MINIO_ACCESS_KEY_ID") {
        Ok(value) => value,
        Err(_) if io::stdin().is_terminal() => prompt("MinIO access key", "kairo")?,
        Err(_) => "kairo".to_owned(),
    };
    let secret_key = match env::var("KAIRO_MINIO_SECRET_ACCESS_KEY") {
        Ok(value) => value,
        Err(_) if io::stdin().is_terminal() => prompt("MinIO secret", &generate_secret()?)?,
        Err(_) => generate_secret()?,
    };
    if access_key.is_empty() || secret_key.is_empty() {
        return Err(SetupError::MinioCredentials);
    }
    if !access_key.chars().all(valid_credential) || !secret_key.chars().all(valid_credential) {
        return Err(SetupError::InvalidMinioCredentials);
    }
    if secret_key.len() < 8 {
        return Err(SetupError::ShortMinioSecret);
    }
    Ok(MinioCredentials {
        access_key,
        secret_key,
    })
}

fn valid_credential(character: char) -> bool {
    character.is_ascii_alphanumeric() || matches!(character, '_' | '-')
}

fn generate_secret() -> Result<String, SetupError> {
    let mut bytes = [0_u8; 24];
    getrandom::fill(&mut bytes).map_err(|source| SetupError::GenerateCredential { source })?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

pub(super) fn write_environment(credentials: &MinioCredentials) -> Result<(), SetupError> {
    let existing = match fs::read_to_string(".env") {
        Ok(existing) => Some(existing),
        Err(source) if source.kind() == io::ErrorKind::NotFound => None,
        Err(source) => return Err(SetupError::ReadEnv { source }),
    };
    if let Some(existing) = existing {
        let access_key = format!("KAIRO_MINIO_ACCESS_KEY_ID={}", credentials.access_key);
        let secret_key = format!("KAIRO_MINIO_SECRET_ACCESS_KEY={}", credentials.secret_key);
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
    writeln!(file, "KAIRO_MINIO_ACCESS_KEY_ID={}", credentials.access_key)
        .and_then(|_| {
            writeln!(
                file,
                "KAIRO_MINIO_SECRET_ACCESS_KEY={}",
                credentials.secret_key
            )
        })
        .and_then(|_| file.sync_all())
        .map_err(|source| SetupError::WriteEnv { source })?;
    restrict_environment_permissions()
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
