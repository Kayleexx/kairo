use std::{fs, io, path::Path};

use kairo_core::{
    Config,
    catalog::{self, ComponentManifest, ComponentSource, MANIFEST_FILE},
};
use thiserror::Error;

const STARTERS: &[(&str, &str, &[u8])] = &[
    (
        "count-primes",
        "Counts prime numbers up to a given upper bound",
        include_bytes!("../../../components/reference/count-primes/component.wasm"),
    ),
    (
        "digit-sum",
        "Sums the decimal digits of a number",
        include_bytes!("../../../components/reference/digit-sum/component.wasm"),
    ),
    (
        "expand-range",
        "Expands a number into a numeric range upper bound (multiplies by 10,000)",
        include_bytes!("../../../components/reference/expand-range/component.wasm"),
    ),
];

#[derive(Debug, Error)]
pub(crate) enum StarterComponentError {
    #[error("failed to create starter Component directory `{path}`")]
    CreateDirectory {
        path: std::path::PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to write starter Component `{path}`")]
    Write {
        path: std::path::PathBuf,
        #[source]
        source: io::Error,
    },
    #[error(transparent)]
    Contract(#[from] kairo_runtime::RuntimeError),
    #[error("failed to serialize starter Component catalog metadata")]
    Serialize(#[source] toml::ser::Error),
}

// bundled into the binary itself so a fresh project has real Components with nothing to add.
pub(crate) fn install(config: Config) -> Result<(), StarterComponentError> {
    // both catalog roots, not just this destination -- avoids shadowing components/reference/.
    let existing = catalog::list(&[Path::new("components"), Path::new("components/reference")]);
    for (name, description, wasm) in STARTERS {
        if existing.iter().any(|entry| entry.name == *name) {
            continue;
        }
        let directory = Path::new("components").join(name);
        let destination = directory.join("component.wasm");
        fs::create_dir_all(&directory).map_err(|source| {
            StarterComponentError::CreateDirectory {
                path: directory.clone(),
                source,
            }
        })?;
        fs::write(&destination, wasm).map_err(|source| StarterComponentError::Write {
            path: destination.clone(),
            source,
        })?;
        let descriptor = kairo_runtime::inspect_contract(&destination, config)?;
        let manifest = ComponentManifest {
            schema: 1,
            name: (*name).to_owned(),
            version: None,
            description: Some((*description).to_owned()),
            hash: descriptor.contract.hash.to_string(),
            source: ComponentSource {
                kind: "bundled".to_owned(),
                reference: (*name).to_owned(),
                resolved_digest: None,
            },
            resources: None,
        };
        let source = toml::to_string_pretty(&manifest).map_err(StarterComponentError::Serialize)?;
        fs::write(directory.join(MANIFEST_FILE), source).map_err(|source| {
            StarterComponentError::Write {
                path: directory.join(MANIFEST_FILE),
                source,
            }
        })?;
    }
    Ok(())
}
