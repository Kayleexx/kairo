use std::path::{Path, PathBuf};

use kairo_core::catalog::ComponentEntry;

use super::NewError;

pub(super) fn choose_component(value: &str) -> Result<PathBuf, NewError> {
    let discovered = discover_components();
    if let Some(entry) = value
        .parse::<usize>()
        .ok()
        .and_then(|index| discovered.get(index.saturating_sub(1)).cloned())
    {
        return Ok(entry.path);
    }
    let path = PathBuf::from(value);
    if path.is_file() {
        return Ok(path);
    }
    resolve_named_component(value)
}

pub(super) fn resolve_named_component(name: &str) -> Result<PathBuf, NewError> {
    list_components()
        .into_iter()
        .find(|entry| entry.name == name)
        .map(|entry| entry.path)
        .ok_or_else(|| NewError::UnknownComponent {
            name: name.to_owned(),
        })
}

pub(super) fn has_available_components() -> bool {
    !list_components().is_empty()
}

pub(super) fn print_no_components_guidance() {
    println!("No Components yet · add one first with `kairo add <path>`.");
}

pub(super) fn discover_components() -> Vec<ComponentEntry> {
    let entries = list_components();
    if !entries.is_empty() {
        println!("available Components");
        for (index, entry) in entries.iter().enumerate() {
            println!("  {} · {}", index + 1, describe(entry));
        }
    }
    entries
}

pub(super) fn list_components() -> Vec<ComponentEntry> {
    kairo_core::catalog::list(&[Path::new("components"), Path::new("components/reference")])
}

pub(super) fn describe(entry: &ComponentEntry) -> String {
    match &entry.description {
        Some(description) => format!("{} · {description}", entry.name),
        None => entry.name.clone(),
    }
}
