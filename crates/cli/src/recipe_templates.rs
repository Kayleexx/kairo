use std::{fs, io, path::Path};

use thiserror::Error;

// each names a Component `starter_components::install` already bundles into the `kairo` binary
// and materializes on `kairo init`, so these resolve on any installed project -- the `source`
// hint below only ever matters if a user has since removed one of those.
const RECIPES: [(&str, &str); 2] = [
    (
        "range-prime-count.yaml",
        "schema: 1\nname: range-prime-count\ntitle: Range prime count\ndescription: Expand a small seed number into a wider numeric range, then count the primes within it.\naccepts: [integer]\nproduces: [integer]\ncomponents:\n  - name: expand-range\n    source: components/expand-range/component.wasm\n  - name: count-primes\n    source: components/count-primes/component.wasm\ndurability: auto\n",
    ),
    (
        "prime-digest.yaml",
        "schema: 1\nname: prime-digest\ntitle: Prime digest\ndescription: Expand a seed number into a range, count the primes within it, then reduce the count to a digit sum.\naccepts: [integer]\nproduces: [integer]\ncomponents:\n  - name: expand-range\n    source: components/expand-range/component.wasm\n  - name: count-primes\n    source: components/count-primes/component.wasm\n  - name: digit-sum\n    source: components/digit-sum/component.wasm\ndurability: auto\n",
    ),
];

#[derive(Debug, Error)]
pub(crate) enum RecipeTemplateError {
    #[error("failed to create project recipe directory")]
    CreateDirectory {
        #[source]
        source: io::Error,
    },
    #[error("failed to install project recipe `{path}`")]
    Write {
        path: String,
        #[source]
        source: io::Error,
    },
}

pub(crate) fn install() -> Result<(), RecipeTemplateError> {
    let directory = Path::new("recipes");
    fs::create_dir_all(directory)
        .map_err(|source| RecipeTemplateError::CreateDirectory { source })?;
    for (name, source) in RECIPES {
        let path = directory.join(name);
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut file) => {
                std::io::Write::write_all(&mut file, source.as_bytes()).map_err(|source| {
                    RecipeTemplateError::Write {
                        path: path.display().to_string(),
                        source,
                    }
                })?
            }
            Err(source) if source.kind() == io::ErrorKind::AlreadyExists => {}
            Err(source) => {
                return Err(RecipeTemplateError::Write {
                    path: path.display().to_string(),
                    source,
                });
            }
        }
    }
    Ok(())
}
