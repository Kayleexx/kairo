use std::{fs, io, path::Path};

use thiserror::Error;

const RECIPES: [(&str, &str); 3] = [
    (
        "video-analysis.yaml",
        "schema: 1\nname: video-analysis\ntitle: Video analysis\ndescription: Decode a video stream and produce analysis output.\naccepts: [video]\nproduces: [analysis]\ncomponents:\n  - name: video-decode\n  - name: video-analyze\ndurability: auto\n",
    ),
    (
        "document-processing.yaml",
        "schema: 1\nname: document-processing\ntitle: Document processing\ndescription: Transform a document and produce a reviewed result.\naccepts: [document]\nproduces: [document]\ncomponents:\n  - name: document-normalize\n  - name: document-review\ndurability: auto\n",
    ),
    (
        "inference-pipeline.yaml",
        "schema: 1\nname: inference-pipeline\ntitle: Inference pipeline\ndescription: Prepare input, run inference, and summarize the result.\naccepts: [input]\nproduces: [result]\ncomponents:\n  - name: inference-prepare\n  - name: inference-run\n  - name: inference-summarize\ndurability: auto\n",
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
