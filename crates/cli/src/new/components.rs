use std::{
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
};

use super::NewError;

pub(super) fn choose_component(value: &str) -> Result<PathBuf, NewError> {
    let discovered = discover_components();
    if let Some(path) = value
        .parse::<usize>()
        .ok()
        .and_then(|index| discovered.get(index.saturating_sub(1)).cloned())
    {
        return Ok(path);
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
    fs::read_dir("components").is_ok_and(|entries| {
        entries.flatten().any(|entry| {
            let path = entry.path();
            path.is_file()
                && matches!(
                    path.extension().and_then(|extension| extension.to_str()),
                    Some("wasm" | "wat" | "wast")
                )
        })
    })
}

pub(super) fn print_no_components_guidance() {
    println!("No components yet · typing a name below creates and builds one automatically.");
}

pub(super) fn discover_components() -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir("components") else {
        return Vec::new();
    };
    let mut paths: Vec<_> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && matches!(
                    path.extension().and_then(|extension| extension.to_str()),
                    Some("wasm" | "wat" | "wast")
                )
        })
        .collect();
    paths.sort();
    if !paths.is_empty() {
        println!("available Components");
        for (index, path) in paths.iter().enumerate() {
            println!("  {} · {}", index + 1, component_label(path));
        }
    }
    paths
}

/// shows the logical name for a component built the standard way (`components/<name>/component.wasm`)
/// instead of a raw path -- a user picking from this list should never need to think about paths.
fn component_label(path: &Path) -> String {
    if path.file_name() == Some(OsStr::new("component.wasm"))
        && let Some(name) = path.parent().and_then(Path::file_name)
    {
        return name.to_string_lossy().into_owned();
    }
    path.display().to_string()
}
