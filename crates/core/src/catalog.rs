use std::{
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

pub const MANIFEST_FILE: &str = "kairo.toml";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ComponentEntry {
    pub path: PathBuf,
    pub name: String,
    pub description: Option<String>,
    pub version: Option<String>,
    pub hash: Option<String>,
    pub source: Option<ComponentSource>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComponentManifest {
    pub schema: u32,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub hash: String,
    pub source: ComponentSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resources: Option<ResourceHints>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComponentSource {
    pub kind: String,
    pub reference: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_digest: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceHints {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_bytes: Option<u64>,
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
                version: None,
                hash: None,
                source: None,
            });
            continue;
        }
        let built = path.join("component.wasm");
        if built.is_file() {
            let legacy_name = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string());
            let manifest = component_manifest(&path.join(MANIFEST_FILE));
            let name = manifest
                .as_ref()
                .map_or_else(|| legacy_name, |manifest| manifest.name.clone());
            let description = manifest
                .as_ref()
                .and_then(|manifest| manifest.description.clone())
                .or_else(|| description_from_manifest(&path.join("Cargo.toml")));
            entries.push(ComponentEntry {
                path: built,
                name,
                description,
                version: manifest
                    .as_ref()
                    .and_then(|manifest| manifest.version.clone()),
                hash: manifest.as_ref().map(|manifest| manifest.hash.clone()),
                source: manifest.map(|manifest| manifest.source),
            });
        }
    }
}

pub fn component_manifest(path: &Path) -> Option<ComponentManifest> {
    let contents = fs::read_to_string(path).ok()?;
    toml::from_str(&contents).ok()
}

fn description_from_manifest(manifest_path: &Path) -> Option<String> {
    let contents = fs::read_to_string(manifest_path).ok()?;
    let manifest: CargoManifest = toml::from_str(&contents).ok()?;
    manifest
        .package
        .description
        .filter(|description| !description.trim().is_empty())
}
