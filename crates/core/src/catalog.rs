use std::{
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
};

use serde::Deserialize;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ComponentEntry {
    pub path: PathBuf,
    pub name: String,
    pub description: Option<String>,
}

#[derive(Deserialize)]
struct CargoManifest {
    package: CargoPackage,
}

#[derive(Deserialize)]
struct CargoPackage {
    #[serde(default)]
    description: Option<String>,
}

// missing roots are skipped, not an error -- a fresh project has no components/reference yet.
pub fn list(roots: &[&Path]) -> Vec<ComponentEntry> {
    let mut entries = Vec::new();
    for root in roots {
        scan(root, &mut entries);
    }
    entries.sort_by(|left, right| left.name.cmp(&right.name));
    entries.dedup_by(|left, right| left.path == right.path);
    entries
}

fn scan(root: &Path, entries: &mut Vec<ComponentEntry>) {
    let Ok(directory) = fs::read_dir(root) else {
        return;
    };
    for item in directory.flatten() {
        let path = item.path();
        if path.is_file()
            && matches!(
                path.extension().and_then(OsStr::to_str),
                Some("wasm" | "wat" | "wast")
            )
        {
            let name = path
                .file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string());
            entries.push(ComponentEntry {
                path,
                name,
                description: None,
            });
            continue;
        }
        let built = path.join("component.wasm");
        if built.is_file() {
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string());
            let description = description_from_manifest(&path.join("Cargo.toml"));
            entries.push(ComponentEntry {
                path: built,
                name,
                description,
            });
        }
    }
}

fn description_from_manifest(manifest_path: &Path) -> Option<String> {
    let contents = fs::read_to_string(manifest_path).ok()?;
    let manifest: CargoManifest = toml::from_str(&contents).ok()?;
    manifest
        .package
        .description
        .filter(|description| !description.trim().is_empty())
}
