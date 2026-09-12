use std::{fs::File, io::Read, path::Path};

use super::WorkflowError;

pub(super) fn read_bounded(path: &Path, max_bytes: usize) -> Result<String, WorkflowError> {
    let file = File::open(path).map_err(|source| WorkflowError::Open {
        path: path.to_path_buf(),
        source,
    })?;
    let limit = u64::try_from(max_bytes).unwrap_or(u64::MAX);
    let mut bytes = Vec::new();
    file.take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|source| WorkflowError::Read {
            path: path.to_path_buf(),
            source,
        })?;
    if bytes.len() > max_bytes {
        return Err(WorkflowError::TooLarge {
            path: path.to_path_buf(),
            max_bytes,
        });
    }
    String::from_utf8(bytes).map_err(|source| WorkflowError::InvalidUtf8 { source })
}
