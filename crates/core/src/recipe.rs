use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};

use serde::Deserialize;
use thiserror::Error;

use crate::Durability;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Recipe {
    pub schema: u32,
    pub name: String,
    pub title: String,
    pub description: String,
    #[serde(default)]
    pub accepts: Vec<String>,
    #[serde(default)]
    pub produces: Vec<String>,
    pub components: Vec<RecipeComponent>,
    #[serde(default)]
    pub durability: RecipeDurability,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecipeComponent {
    pub name: String,
    #[serde(default)]
    pub version: Option<String>,
    /// an optional real local path or OCI reference shown when this project has not registered
    /// the Component yet. recipe execution never resolves it implicitly.
    #[serde(default)]
    pub source: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RecipeDurability {
    Ephemeral,
    Required,
    #[default]
    Auto,
}

impl From<RecipeDurability> for Durability {
    fn from(value: RecipeDurability) -> Self {
        match value {
            RecipeDurability::Ephemeral => Self::Ephemeral,
            RecipeDurability::Required => Self::Required,
            RecipeDurability::Auto => Self::Auto,
        }
    }
}

#[derive(Debug, Error)]
pub enum RecipeError {
    #[error("failed to open recipe `{path}`")]
    Open {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("recipe `{path}` exceeds the {max_bytes}-byte limit")]
    TooLarge { path: PathBuf, max_bytes: usize },
    #[error("recipe `{path}` is invalid")]
    Parse {
        path: PathBuf,
        #[source]
        source: yaml_serde::Error,
    },
    #[error("recipe schema {found} is unsupported; expected schema 1")]
    Schema { found: u32 },
    #[error("recipe must contain at least one Component")]
    Empty,
    #[error("recipe Component `{component}` has an invalid source reference")]
    InvalidComponentSource { component: String },
}

impl Recipe {
    pub fn load(path: &Path, max_bytes: usize) -> Result<Self, RecipeError> {
        let file = File::open(path).map_err(|source| RecipeError::Open {
            path: path.to_path_buf(),
            source,
        })?;
        let mut source = String::new();
        file.take(
            u64::try_from(max_bytes)
                .unwrap_or(u64::MAX)
                .saturating_add(1),
        )
        .read_to_string(&mut source)
        .map_err(|source| RecipeError::Open {
            path: path.to_path_buf(),
            source,
        })?;
        if source.len() > max_bytes {
            return Err(RecipeError::TooLarge {
                path: path.to_path_buf(),
                max_bytes,
            });
        }
        let recipe: Self = yaml_serde::from_str(&source).map_err(|source| RecipeError::Parse {
            path: path.to_path_buf(),
            source,
        })?;
        if recipe.schema != 1 {
            return Err(RecipeError::Schema {
                found: recipe.schema,
            });
        }
        if recipe.components.is_empty() {
            return Err(RecipeError::Empty);
        }
        if let Some(component) = recipe.components.iter().find(|component| {
            component.source.as_ref().is_some_and(|source| {
                source.trim().is_empty()
                    || source.len() > 512
                    || source.chars().any(char::is_control)
            })
        }) {
            return Err(RecipeError::InvalidComponentSource {
                component: component.name.clone(),
            });
        }
        Ok(recipe)
    }
}

pub fn discover(root: &Path, max_bytes: usize) -> Vec<(PathBuf, Recipe)> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut recipes = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .and_then(|value| value.to_str())
                .is_some_and(|value| matches!(value, "yaml" | "yml"))
        })
        .filter_map(|path| {
            Recipe::load(&path, max_bytes)
                .ok()
                .map(|recipe| (path, recipe))
        })
        .collect::<Vec<_>>();
    recipes.sort_by(|left, right| left.1.name.cmp(&right.1.name));
    recipes
}
