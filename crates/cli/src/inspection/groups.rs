use std::{
    fs,
    path::{Path, PathBuf},
};

use kairo_runtime::{CellInspection, JournalError, inspect_cell};

use crate::state::is_group_journal_stem;

// only the *last* ExecutionGroup's journal ever reaches `Completed`; this merges every group's
// journal into the single view callers expect, with the last group's status authoritative.
pub(crate) fn inspect_aggregated(path: &Path) -> Result<CellInspection, JournalError> {
    let mut inspection = inspect_cell(path)?;
    for group_path in sibling_groups(path) {
        let group = inspect_cell(&group_path)?;
        inspection.components.extend(group.components);
        inspection.status = group.status;
        inspection.recovery_duration_us = group
            .recovery_duration_us
            .or(inspection.recovery_duration_us);
        inspection.metadata_complete = inspection.metadata_complete && group.metadata_complete;
    }
    Ok(inspection)
}

pub(crate) fn sibling_groups(base: &Path) -> Vec<PathBuf> {
    let (Some(stem), Some(parent)) = (
        base.file_stem().and_then(|stem| stem.to_str()),
        base.parent(),
    ) else {
        return Vec::new();
    };
    let prefix = format!("{stem}.group-");
    let Ok(entries) = fs::read_dir(parent) else {
        return Vec::new();
    };
    let mut groups: Vec<(usize, PathBuf)> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name();
            let file_stem = name.to_str()?.strip_suffix(".db")?;
            if !is_group_journal_stem(file_stem) {
                return None;
            }
            let index: usize = file_stem.strip_prefix(&prefix)?.parse().ok()?;
            Some((index, entry.path()))
        })
        .collect();
    groups.sort_by_key(|(index, _)| *index);
    groups.into_iter().map(|(_, path)| path).collect()
}
