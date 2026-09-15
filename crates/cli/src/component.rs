use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

use kairo_core::Config;
use serde::Deserialize;
use thiserror::Error;

use crate::{args::ComponentCommand, print_valid, validation};

pub(crate) fn dispatch(command: ComponentCommand, config: Config) -> crate::Result<()> {
    match command {
        ComponentCommand::New { name } => {
            let directory = new(&name)?;
            print_valid(format!(
                "component · {}\nnext: edit src/lib.rs, then run `kairo component build {}`",
                directory.display(),
                directory.display()
            ));
        }
        ComponentCommand::Build { path } => {
            let component_path = build(&path)?;
            print_valid(format!("component · {}", component_path.display()));
        }
        ComponentCommand::Check { path } => validation::component(&path, config)?,
    }
    Ok(())
}

const WIT_BINDGEN_VERSION: &str = "0.61.1";
const WIT_WORLD: &str = "package kairo:example@0.1.0;\n\nworld value-stage {\n    export run: async func(input: list<u8>) -> result<list<u8>, string>;\n}\n";
const LIB_RS: &str = "wit_bindgen::generate!({ world: \"value-stage\", path: \"wit\" });\n\nstruct Component;\n\nimpl Guest for Component {\n    async fn run(input: Vec<u8>) -> Result<Vec<u8>, String> {\n        // replace with real logic -- input/output are raw bytes; decode/encode them however\n        // this component's workflow expects (JSON, msgpack, a document, ...). Kairo itself\n        // never inspects them.\n        Ok(input)\n    }\n}\n\nexport!(Component);\n";

#[derive(Debug, Error)]
pub(crate) enum ComponentError {
    #[error(
        "component name must be 1-64 lowercase letters, digits, `-`, or `_`, starting with a letter"
    )]
    InvalidName,
    #[error("`components/{name}` already exists")]
    AlreadyExists { name: String },
    #[error("failed to create component directory `{path}`")]
    CreateDirectory {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to write `{path}`")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("`{path}` is not a component project: missing Cargo.toml")]
    MissingManifest { path: PathBuf },
    #[error("failed to read `{path}`")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse `{path}`")]
    InvalidManifest {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
    #[error("`{path}` has no `[package] name`")]
    MissingPackageName { path: PathBuf },
    #[error("failed to run `{command}`; is it installed and on PATH?")]
    ToolNotFound { command: &'static str },
    #[error("failed to run `{command}`")]
    Spawn {
        command: &'static str,
        #[source]
        source: std::io::Error,
    },
    #[error("`{step}` failed:\n{message}")]
    ToolFailed { step: &'static str, message: String },
}

#[derive(Deserialize)]
struct CargoManifest {
    package: CargoPackage,
}

#[derive(Deserialize)]
struct CargoPackage {
    name: String,
}

pub(crate) fn new(name: &str) -> Result<PathBuf, ComponentError> {
    if !valid_name(name) {
        return Err(ComponentError::InvalidName);
    }
    let directory = Path::new("components").join(name);
    if directory.exists() {
        return Err(ComponentError::AlreadyExists {
            name: name.to_owned(),
        });
    }
    create_dir(&directory.join("wit"))?;
    create_dir(&directory.join("src"))?;
    write(&directory.join("Cargo.toml"), &cargo_toml(name))?;
    write(&directory.join("wit/workflow.wit"), WIT_WORLD)?;
    write(&directory.join("src/lib.rs"), LIB_RS)?;
    Ok(directory)
}

/// orchestrates the same `cargo build` -> `wasm-tools component new` -> `wasm-tools validate`
/// pipeline already used by `components/reference/build.sh` -- never a replacement for Cargo.
pub(crate) fn build(path: &Path) -> Result<PathBuf, ComponentError> {
    let manifest_path = path.join("Cargo.toml");
    let manifest = fs::read_to_string(&manifest_path).map_err(|source| {
        if source.kind() == std::io::ErrorKind::NotFound {
            ComponentError::MissingManifest {
                path: manifest_path.clone(),
            }
        } else {
            ComponentError::Read {
                path: manifest_path.clone(),
                source,
            }
        }
    })?;
    let document: CargoManifest =
        toml::from_str(&manifest).map_err(|source| ComponentError::InvalidManifest {
            path: manifest_path.clone(),
            source,
        })?;
    if document.package.name.is_empty() {
        return Err(ComponentError::MissingPackageName {
            path: manifest_path,
        });
    }
    let module_name = document.package.name.replace('-', "_");

    let build = Command::new("cargo")
        .args(["build", "--release", "--target", "wasm32-unknown-unknown"])
        .current_dir(path)
        .output()
        .map_err(|source| spawn_error("cargo", source))?;
    check(&build, "cargo build")?;

    let wasm_path = path
        .join("target/wasm32-unknown-unknown/release")
        .join(format!("{module_name}.wasm"));
    let component_path = path.join("component.wasm");

    let componentize = Command::new("wasm-tools")
        .arg("component")
        .arg("new")
        .arg(&wasm_path)
        .arg("-o")
        .arg(&component_path)
        .output()
        .map_err(|source| spawn_error("wasm-tools", source))?;
    check(&componentize, "wasm-tools component new")?;

    let validate = Command::new("wasm-tools")
        .args(["validate", "--features", "cm-async"])
        .arg(&component_path)
        .output()
        .map_err(|source| spawn_error("wasm-tools", source))?;
    check(&validate, "wasm-tools validate")?;

    Ok(component_path)
}

fn spawn_error(command: &'static str, source: std::io::Error) -> ComponentError {
    if source.kind() == std::io::ErrorKind::NotFound {
        ComponentError::ToolNotFound { command }
    } else {
        ComponentError::Spawn { command, source }
    }
}

fn check(output: &Output, step: &'static str) -> Result<(), ComponentError> {
    if output.status.success() {
        return Ok(());
    }
    Err(ComponentError::ToolFailed {
        step,
        message: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
    })
}

fn create_dir(path: &Path) -> Result<(), ComponentError> {
    fs::create_dir_all(path).map_err(|source| ComponentError::CreateDirectory {
        path: path.to_path_buf(),
        source,
    })
}

fn write(path: &Path, contents: &str) -> Result<(), ComponentError> {
    fs::write(path, contents).map_err(|source| ComponentError::Write {
        path: path.to_path_buf(),
        source,
    })
}

fn cargo_toml(name: &str) -> String {
    format!(
        "[workspace]\n\n[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[lib]\ncrate-type = [\"cdylib\"]\n\n[dependencies]\nwit-bindgen = \"{WIT_BINDGEN_VERSION}\"\n"
    )
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .next()
            .is_some_and(|first| first.is_ascii_lowercase())
        && name.chars().all(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || matches!(character, '-' | '_')
        })
}
