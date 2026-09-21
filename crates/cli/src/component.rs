use std::{
    fs,
    path::{Path, PathBuf},
};

use kairo_core::{
    Config,
    catalog::{self, ComponentManifest, ComponentSource, MANIFEST_FILE},
};
use kairo_tui::scaffold::{self, ScaffoldError};
use thiserror::Error;

use crate::{args::ComponentCommand, print_valid, validation};

mod oci;

#[derive(Debug, Error)]
pub(crate) enum ComponentError {
    #[error(transparent)]
    Scaffold(#[from] ScaffoldError),
    #[error(transparent)]
    Runtime(#[from] kairo_runtime::RuntimeError),
    #[error(transparent)]
    Oci(#[from] oci::OciError),
    #[error("invalid Component name `{name}`; use lowercase letters, digits, `-`, or `_`")]
    InvalidName { name: String },
    #[error("Component name `{name}` is already registered with different content")]
    NameConflict { name: String },
    #[error("Component description must be at most 200 printable characters")]
    InvalidDescription,
    #[error("failed to create Component catalog directory `{path}`")]
    CreateDirectory {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to vendor Component from `{source_path}` to `{destination}`")]
    Copy {
        source_path: PathBuf,
        destination: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to serialize Component catalog metadata")]
    Serialize(#[source] toml::ser::Error),
    #[error("failed to write Component catalog metadata `{path}`")]
    WriteManifest {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("Component `{name}` is not registered")]
    Missing { name: String },
}

pub(crate) fn dispatch(command: ComponentCommand, config: Config) -> crate::Result<()> {
    match command {
        ComponentCommand::New { name } => {
            let directory = new(&name)?;
            print_valid(format!(
                "component · {}\nnext: edit src/lib.rs (and Cargo.toml's `description`, so it \
                 shows up in `kairo new`), then run `kairo component build {}`",
                directory.display(),
                directory.display()
            ));
        }
        ComponentCommand::Build { path } => {
            let component_path = build(&path)?;
            print_valid(format!("component · {}", component_path.display()));
        }
        ComponentCommand::Check { path } => validation::component(&path, config)?,
        ComponentCommand::Show { name } => show(&name, config)?,
    }
    Ok(())
}

pub(crate) async fn add(
    source: &str,
    name: Option<&str>,
    version: Option<&str>,
    description: Option<&str>,
    config: Config,
) -> Result<(), ComponentError> {
    let path = Path::new(source);
    if path.is_file() || !looks_like_oci(source) {
        return add_local(path, name, version, description, config);
    }
    let resolved = oci::resolve(source, config).await?;
    let display_digest = resolved.digest.clone();
    let descriptor = add_vendored(
        &resolved.path,
        name,
        resolved.version.as_deref().or(version),
        description,
        resolved.source,
        config,
    )?;
    let component_name = name.map_or_else(|| default_oci_name(source), str::to_owned);
    println!("added {component_name}");
    if let Some(version) = descriptor.version.as_deref().or(version) {
        println!("version: {version}");
    }
    println!("digest: {display_digest}");
    println!("contract: {}", descriptor.contract.shape());
    Ok(())
}

fn add_local(
    path: &Path,
    name: Option<&str>,
    version: Option<&str>,
    description: Option<&str>,
    config: Config,
) -> Result<(), ComponentError> {
    let descriptor = add_vendored(
        path,
        name,
        version,
        description,
        ComponentSource {
            kind: "local".to_owned(),
            reference: path.display().to_string(),
            resolved_digest: None,
        },
        config,
    )?;
    let name = name.map_or_else(
        || {
            path.file_stem().map_or_else(
                || "component".to_owned(),
                |name| name.to_string_lossy().into_owned(),
            )
        },
        str::to_owned,
    );
    print_valid(format!(
        "Component · {name} · {} · {}",
        descriptor.contract.shape(),
        descriptor.contract.hash
    ));
    Ok(())
}

fn add_vendored(
    path: &Path,
    name: Option<&str>,
    version: Option<&str>,
    description: Option<&str>,
    source: ComponentSource,
    config: Config,
) -> Result<kairo_runtime::ComponentDescriptor, ComponentError> {
    if description
        .is_some_and(|value| value.chars().count() > 200 || value.chars().any(char::is_control))
    {
        return Err(ComponentError::InvalidDescription);
    }
    let descriptor = kairo_runtime::inspect_contract(path, config)?;
    let name = name.map_or_else(
        || {
            path.file_stem().map_or_else(
                || "component".to_owned(),
                |name| name.to_string_lossy().into_owned(),
            )
        },
        str::to_owned,
    );
    if !valid_name(&name) {
        return Err(ComponentError::InvalidName { name });
    }
    let directory = Path::new("components").join(&name);
    let destination = directory.join("component.wasm");
    let already_vendored = destination.is_file();
    let existing_manifest = already_vendored
        .then(|| catalog::component_manifest(&directory.join(MANIFEST_FILE)))
        .flatten();
    if already_vendored {
        let existing = kairo_runtime::inspect_contract(&destination, config)?;
        if existing.contract.hash != descriptor.contract.hash {
            return Err(ComponentError::NameConflict { name });
        }
    }
    fs::create_dir_all(&directory).map_err(|source| ComponentError::CreateDirectory {
        path: directory.clone(),
        source,
    })?;
    if !already_vendored {
        let temporary_component = directory.join(format!(".component-{}.tmp", std::process::id()));
        fs::copy(path, &temporary_component).map_err(|source| ComponentError::Copy {
            source_path: path.to_path_buf(),
            destination: temporary_component.clone(),
            source,
        })?;
        fs::rename(&temporary_component, &destination).map_err(|source| ComponentError::Copy {
            source_path: temporary_component,
            destination: destination.clone(),
            source,
        })?;
    }
    let manifest = ComponentManifest {
        schema: 1,
        name: name.clone(),
        version: descriptor
            .version
            .clone()
            .or_else(|| version.map(str::to_owned))
            .or_else(|| {
                existing_manifest
                    .as_ref()
                    .and_then(|manifest| manifest.version.clone())
            }),
        description: description
            .map(str::to_owned)
            .or_else(|| existing_manifest.and_then(|manifest| manifest.description)),
        hash: descriptor.contract.hash.to_string(),
        source,
        resources: None,
    };
    let manifest_source = toml::to_string_pretty(&manifest).map_err(ComponentError::Serialize)?;
    let manifest_path = directory.join(MANIFEST_FILE);
    let temporary_manifest = directory.join(format!(".{MANIFEST_FILE}-{}.tmp", std::process::id()));
    fs::write(&temporary_manifest, manifest_source).map_err(|source| {
        ComponentError::WriteManifest {
            path: temporary_manifest.clone(),
            source,
        }
    })?;
    fs::rename(&temporary_manifest, &manifest_path).map_err(|source| {
        ComponentError::WriteManifest {
            path: manifest_path,
            source,
        }
    })?;
    Ok(descriptor)
}

pub(crate) fn list(config: Config) {
    let entries = catalog::list(&[Path::new("components"), Path::new("components/reference")]);
    if entries.is_empty() {
        println!("No Components registered. Add one with `kairo add <path>`.");
        return;
    }
    for entry in entries {
        let detail = kairo_runtime::inspect_contract(&entry.path, config)
            .map(|descriptor| descriptor.contract.shape())
            .unwrap_or("unsupported contract");
        let version = entry
            .version
            .as_deref()
            .map_or(String::new(), |value| format!(" {value}"));
        let description = entry
            .description
            .as_deref()
            .map(|description| format!(" · {description}"))
            .unwrap_or_default();
        println!("{}{version}{description} · {detail}", entry.name);
    }
}

fn show(name: &str, config: Config) -> Result<(), ComponentError> {
    let entry = catalog::list(&[Path::new("components"), Path::new("components/reference")])
        .into_iter()
        .find(|entry| entry.name == name)
        .ok_or_else(|| ComponentError::Missing {
            name: name.to_owned(),
        })?;
    let descriptor = kairo_runtime::inspect_contract(&entry.path, config)?;
    println!("Component  {}", entry.name);
    if let Some(description) = entry.description.as_deref() {
        println!("Does       {description}");
    }
    if let Some(version) = entry.version.as_deref().or(descriptor.version.as_deref()) {
        println!("Version    {version}");
    }
    println!("Contract   {}", descriptor.contract.shape());
    if let Some(package) = descriptor.package.as_deref() {
        println!("WIT package {package}");
    }
    println!("WIT world  {}", descriptor.world);
    println!("Hash       {}", descriptor.contract.hash);
    println!(
        "Source     {}",
        entry
            .source
            .as_ref()
            .map_or("legacy project component", |source| source
                .reference
                .as_str())
    );
    if let Some(digest) = entry
        .source
        .as_ref()
        .and_then(|source| source.resolved_digest.as_deref())
    {
        println!("Digest     {digest}");
    }
    if !descriptor.imports.is_empty() {
        println!("Requires   {}", descriptor.imports.join(", "));
    }
    if !descriptor.exports.is_empty() {
        println!("Exports    {}", descriptor.exports.join(", "));
    }
    if let Some(memory) = entry
        .path
        .parent()
        .and_then(|directory| catalog::component_manifest(&directory.join(MANIFEST_FILE)))
        .and_then(|manifest| manifest.resources)
        .and_then(|resources| resources.memory_bytes)
    {
        println!("Memory     {memory} bytes");
    }
    Ok(())
}

pub(crate) fn new(name: &str) -> Result<std::path::PathBuf, ComponentError> {
    Ok(scaffold::new(name)?)
}

pub(crate) fn build(path: &Path) -> Result<std::path::PathBuf, ComponentError> {
    Ok(scaffold::build(path)?)
}

pub(crate) fn valid_name(name: &str) -> bool {
    scaffold::valid_name(name)
}

fn looks_like_oci(source: &str) -> bool {
    let Some((registry, _)) = source.split_once('/') else {
        return false;
    };
    !source.starts_with('.')
        && !source.starts_with('/')
        && (registry == "localhost" || registry.contains('.') || registry.contains(':'))
}

fn default_oci_name(source: &str) -> String {
    source
        .rsplit('/')
        .next()
        .unwrap_or("component")
        .split('@')
        .next()
        .unwrap_or("component")
        .split(':')
        .next()
        .unwrap_or("component")
        .to_owned()
}
