use std::{
    fs,
    path::{Path, PathBuf},
};

use thiserror::Error;

const STATE_DIRECTORY: &str = ".kairo";

#[derive(Debug, Error)]
pub enum LocalStateError {
    #[error("failed to read local run directory `{path}")]
    ReadDirectory {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to read an entry in `{path}")]
    ReadEntry {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalCell {
    pub name: String,
    pub path: PathBuf,
}

pub fn discover_cells() -> Result<Vec<LocalCell>, LocalStateError> {
    discover_cells_in(Path::new(STATE_DIRECTORY))
}

pub fn discover_cells_in(directory: &Path) -> Result<Vec<LocalCell>, LocalStateError> {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => {
            return Err(LocalStateError::ReadDirectory {
                path: directory.to_path_buf(),
                source,
            });
        }
    };
    let mut cells = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| LocalStateError::ReadEntry {
            path: directory.to_path_buf(),
            source,
        })?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|source| LocalStateError::ReadEntry {
                path: path.clone(),
                source,
            })?;
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
