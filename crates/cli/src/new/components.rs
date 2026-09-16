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

/// resolves a logical component name to the exact path `kairo component build` writes to. If it
/// already exists, reuses it; otherwise scaffolds and builds it automatically, so
/// `kairo workflow new <name> <component-names...>` never requires a user to have already run
/// `component new`/`component build` by hand -- the workflow command is the whole authoring path.
pub(super) fn resolve_named_component(name: &str) -> Result<PathBuf, NewError> {
    let path = Path::new("components").join(name).join("component.wasm");
    if path.is_file() {
        return Ok(path);
    }
    let directory = Path::new("components").join(name);
    if !directory.exists() {
        crate::status("36", "→", &format!("scaffolding {name}"));
        crate::component::new(name)?;
    }
    crate::status("36", "→", &format!("building {name}"));
    let built = crate::component::build(&directory)?;
    crate::status("32", "✓", &format!("component · {}", built.display()));
    Ok(built)
}

pub(super) fn has_available_components() -> bool {
    !list_components().is_empty()
}

pub(super) fn print_no_components_guidance() {
    println!("No components yet · typing a name below creates and builds one automatically.");
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
