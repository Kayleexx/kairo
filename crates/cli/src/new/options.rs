use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

use kairo_core::DraftWait;

use super::NewError;

pub(super) fn default_step_names(components: &[PathBuf]) -> Vec<String> {
    let mut used = HashSet::new();
    components
        .iter()
        .enumerate()
        .map(|(index, path)| {
            let base = default_step_name(path, index);
            let mut name = base.clone();
            let mut suffix = 2;
            while used.contains(&name) {
                name = format!("{base}-{suffix}");
                suffix += 1;
            }
            used.insert(name.clone());
            name
        })
        .collect()
}

pub(super) fn default_step_name(path: &Path, index: usize) -> String {
    match path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
    {
        Some(stem) if stem != "component" && !stem.is_empty() => stem,
        _ => path.parent().and_then(Path::file_name).map_or_else(
            || format!("step-{}", index + 1),
            |name| name.to_string_lossy().into_owned(),
        ),
    }
}

pub(super) fn parse_wait(value: Option<String>) -> Result<Option<DraftWait>, NewError> {
    let Some(value) = value else { return Ok(None) };
    if let Some(milliseconds) = value.strip_prefix("timer:") {
        return milliseconds
            .parse::<u64>()
            .map(DraftWait::Timer)
            .map(Some)
            .map_err(|_| NewError::InvalidWait);
    }
    if let Some(signal) = value.strip_prefix("signal:") {
        if signal.is_empty() || signal.chars().any(char::is_control) {
            return Err(NewError::InvalidWait);
        }
        return Ok(Some(DraftWait::Signal(signal.to_owned())));
    }
    Err(NewError::InvalidWait)
}

pub(super) fn parse_effect(value: Option<String>) -> Result<Option<String>, NewError> {
    let Some(value) = value else { return Ok(None) };
    if value.is_empty()
        || value.len() > 64
        || !value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        return Err(NewError::MissingValue {
            field: "effect operation".to_owned(),
        });
    }
    Ok(Some(value))
}
