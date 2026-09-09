use std::{
    fs,
    path::{Path, PathBuf},
};

use kairo_core::{Config, Workflow};
use thiserror::Error;

const MAX_FILES: usize = 256;

#[derive(Debug, Error)]
pub(crate) enum DiscoveryError {
    #[error("workflow name `{name}` is ambiguous; use one of: {paths}")]
    Ambiguous { name: String, paths: String },
}

pub(crate) fn resolve(path: &Path, config: Config) -> Result<PathBuf, DiscoveryError> {
    if path.exists() || is_yaml(path) {
        return Ok(path.to_path_buf());
    }
    let sibling = PathBuf::from(format!("{}.yaml", path.display()));
    if sibling.is_file() {
        return Ok(sibling);
    }
    let name = path.to_string_lossy();
    let mut matches = Vec::new();
    let mut remaining = MAX_FILES;
    for root in [Path::new("workflows"), Path::new("demos")] {
        collect(root, &mut matches, &mut remaining, config, name.as_ref());
    }
    match matches.len() {
        1 => Ok(matches.remove(0)),
        0 => Ok(path.to_path_buf()),
        _ => Err(DiscoveryError::Ambiguous {
            name: name.into_owned(),
            paths: matches
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", "),
        }),
    }
}

fn collect(
    directory: &Path,
    matches: &mut Vec<PathBuf>,
    remaining: &mut usize,
    config: Config,
    name: &str,
) {
    if *remaining == 0 {
        return;
    }
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        *remaining = remaining.saturating_sub(1);
        let path = entry.path();
        if path.is_dir() {
            collect(&path, matches, remaining, config, name);
        } else if is_yaml(&path)
            && Workflow::load(&path, config.max_workflow_bytes, config.max_workflow_steps)
                .is_ok_and(|workflow| workflow.name() == name)
        {
            matches.push(path);
        }
        if *remaining == 0 {
            return;
        }
    }
}

fn is_yaml(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("yaml") || extension.eq_ignore_ascii_case("yml")
        })
}
