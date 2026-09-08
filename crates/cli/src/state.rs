use std::{
    fs,
    path::{Path, PathBuf},
    process,
    sync::atomic::{AtomicU64, Ordering},
};

use thiserror::Error;

const STATE_DIRECTORY: &str = ".kairo";
static NEXT_RUN: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Error)]
pub(crate) enum StateError {
    #[error("failed to read local run directory `{path}`")]
    ReadDirectory {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to read an entry in `{path}`")]
    ReadEntry {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("no runs found; run a workflow first")]
    NoCells,
    #[error("failed to read run metadata for `{path}`")]
    Metadata {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid run name `{id}`; use 1-64 letters, numbers, `_`, or `-`")]
    InvalidRunName { id: String },
}

pub(crate) struct LocalCell {
    pub(crate) name: String,
    pub(crate) path: PathBuf,
}

pub(crate) fn resolve_run(
    raw: Option<&Path>,
    cell: Option<&str>,
    workflow_name: &str,
) -> Result<Option<PathBuf>, StateError> {
    if let Some(id) = cell {
        validate_id(id)?;
        return Ok(Some(
            PathBuf::from(STATE_DIRECTORY).join(format!("{id}.db")),
        ));
    }
    Ok(match raw {
        None => None,
        Some(path) if path == Path::new("-") => {
            Some(PathBuf::from(STATE_DIRECTORY).join(format!("{}.db", sanitize(workflow_name))))
        }
        Some(path) => Some(path.to_path_buf()),
    })
}

pub(crate) fn generated_run(workflow_name: &str) -> PathBuf {
    let sequence = NEXT_RUN.fetch_add(1, Ordering::Relaxed);
    PathBuf::from(STATE_DIRECTORY).join(format!(
        "{}-{}-{sequence}.db",
        sanitize(workflow_name),
        process::id()
    ))
}

pub(crate) fn discover() -> Result<Vec<LocalCell>, StateError> {
    let directory = Path::new(STATE_DIRECTORY);
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => {
            return Err(StateError::ReadDirectory {
                path: directory.to_path_buf(),
                source,
            });
        }
    };
    let mut cells = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| StateError::ReadEntry {
            path: directory.to_path_buf(),
            source,
        })?;
        let file_type = entry.file_type().map_err(|source| StateError::ReadEntry {
            path: entry.path(),
            source,
        })?;
        let path = entry.path();
        if !file_type.is_file()
            || !path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("db"))
        {
            continue;
        }
        let name = path.file_stem().map_or_else(
            || path.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        );
        cells.push(LocalCell { name, path });
    }
    cells.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(cells)
}

pub(crate) fn select(requested: Option<&Path>) -> Result<LocalCell, StateError> {
    if let Some(requested) = requested {
        let path = if requested.components().count() == 1 && requested.extension().is_none() {
            PathBuf::from(STATE_DIRECTORY)
                .join(requested)
                .with_extension("db")
        } else {
            requested.to_path_buf()
        };
        let name = path.file_stem().map_or_else(
            || path.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        );
        return Ok(LocalCell { name, path });
    }
    let mut runs = discover()?;
    match runs.len() {
        0 => Err(StateError::NoCells),
        1 => Ok(runs.remove(0)),
        _ => latest(&mut runs),
    }
}

fn latest(runs: &mut Vec<LocalCell>) -> Result<LocalCell, StateError> {
    let mut latest = runs.remove(0);
    let mut latest_modified = modified(&latest)?;
    for run in runs.drain(..) {
        let candidate_modified = modified(&run)?;
        if candidate_modified > latest_modified {
            latest = run;
            latest_modified = candidate_modified;
        }
    }
    Ok(latest)
}

fn modified(run: &LocalCell) -> Result<std::time::SystemTime, StateError> {
    fs::metadata(&run.path)
        .and_then(|metadata| metadata.modified())
        .map_err(|source| StateError::Metadata {
            path: run.path.clone(),
            source,
        })
}

fn validate_id(id: &str) -> Result<(), StateError> {
    if id.is_empty()
        || id.len() > 64
        || !id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        return Err(StateError::InvalidRunName { id: id.to_owned() });
    }
    Ok(())
}

fn sanitize(name: &str) -> String {
    name.chars()
        .map(|character| {
            if character.is_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect()
}
