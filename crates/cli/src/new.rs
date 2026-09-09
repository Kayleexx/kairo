use std::{fs::OpenOptions, io::Write, path::PathBuf};

use thiserror::Error;

#[derive(Debug, Error)]
pub(crate) enum NewError {
    #[error("invalid workflow name `{name}`; use 1-64 letters, numbers, `_`, or `-`")]
    InvalidName { name: String },
    #[error("component path must be valid UTF-8 and contain no control characters")]
    InvalidComponentPath,
    #[error("failed to create workflow `{path}`")]
    Create {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to write workflow `{path}`")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

pub(crate) fn workflow(
    name: &str,
    component: &std::path::Path,
    input: u32,
) -> Result<(), NewError> {
    valid_name(name)?;
    let component = component
        .to_str()
        .filter(|path| !path.chars().any(char::is_control))
        .ok_or(NewError::InvalidComponentPath)?
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    let path = PathBuf::from(format!("{name}.yaml"));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|source| NewError::Create {
            path: path.clone(),
            source,
        })?;
    write!(
        file,
        "workflow: {name}\ninput: {input}\n\nsteps:\n  - name: run\n    component: \"{component}\"\n\nedges: []\n"
    )
    .map_err(|source| NewError::Write {
        path: path.clone(),
        source,
    })?;
    println!(
        "created {}\nnext · kairo run {}",
        path.display(),
        path.display()
    );
    Ok(())
}

fn valid_name(name: &str) -> Result<(), NewError> {
    if name.is_empty()
        || name.len() > 64
        || !name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        return Err(NewError::InvalidName {
            name: name.to_owned(),
        });
    }
    Ok(())
}
