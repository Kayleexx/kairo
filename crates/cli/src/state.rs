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
    #[error("failed to read local Cell directory `{path}`")]
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
    #[error("no Cells found; run a workflow with `--cell <id>` or `--state` first")]
    NoCells,
    #[error("more than one Cell exists ({cells}); run `kairo inspect <cell>`")]
    CellRequired { cells: String },
    #[error("invalid Cell ID `{id}`; use 1-64 letters, numbers, `_`, or `-`")]
    InvalidCellId { id: String },
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
    let mut cells = discover()?;
    match cells.len() {
        0 => Err(StateError::NoCells),
        1 => Ok(cells.remove(0)),
        _ => Err(StateError::CellRequired {
            cells: cells
                .iter()
                .map(|cell| cell.name.as_str())
                .collect::<Vec<_>>()
                .join(", "),
        }),
    }
}

fn validate_id(id: &str) -> Result<(), StateError> {
    if id.is_empty()
        || id.len() > 64
        || !id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        return Err(StateError::InvalidCellId { id: id.to_owned() });
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
