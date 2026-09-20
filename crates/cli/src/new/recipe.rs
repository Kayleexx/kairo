use std::{
    fs::OpenOptions,
    io::Write as _,
    path::{Path, PathBuf},
};

use kairo_core::{
    Config, DraftStep, Durability, Workflow, WorkflowDraft,
    catalog::{self, ComponentEntry},
    recipe::{self, Recipe},
};
use kairo_runtime::Runtime;

use super::{CreatedWorkflow, NewError, default_step_names, valid_name};

pub(super) fn list(config: Config) {
    let recipes = recipe::discover(Path::new("recipes"), config.max_workflow_bytes);
    if recipes.is_empty() {
        println!("No project recipes found in `recipes/`.");
        return;
    }
    for (_, recipe) in recipes {
        println!(
            "{} · {} · {}",
            recipe.name, recipe.title, recipe.description
        );
    }
}

pub(super) fn from_recipe(
    name: Option<String>,
    recipe_name: &str,
    config: Config,
) -> Result<CreatedWorkflow, NewError> {
    let (_, recipe) = recipe::discover(Path::new("recipes"), config.max_workflow_bytes)
        .into_iter()
        .find(|(_, recipe)| recipe.name == recipe_name)
        .ok_or_else(|| NewError::MissingRecipe {
            recipe: recipe_name.to_owned(),
        })?;
    let name = name.unwrap_or_else(|| recipe.name.clone());
    valid_name(&name)?;
    let catalog = catalog::list(&[Path::new("components"), Path::new("components/reference")]);
    let selected = resolve_components(&recipe, &catalog, config)?;
    let step_names = default_step_names(
        &selected
            .iter()
            .map(|(entry, _)| entry.path.clone())
            .collect::<Vec<_>>(),
    );
    let final_role = selected
        .last()
        .map(|(_, descriptor)| descriptor.contract.role)
        .ok_or(NewError::NoSteps)?;
    if !final_role.finishable(selected.len()) {
        return Err(NewError::RecipeConnection {
            recipe: recipe.name,
            previous: final_role.shape().to_owned(),
            next: "a terminal Component".to_owned(),
        });
    }
    let output = (final_role == kairo_core::ComponentRole::StreamOutput)
        .then(|| (format!("{name}.bin"), "application/octet-stream".to_owned()));
    let draft = WorkflowDraft {
        name: name.clone(),
        description: Some(recipe.description),
        accepts: recipe.accepts,
        produces: recipe.produces,
        mode: final_role.mode(),
        scalar_input: 0,
        steps: selected
            .into_iter()
            .zip(step_names)
            .map(|((entry, descriptor), step_name)| DraftStep {
                name: step_name,
                component: entry.path,
                hash: Some(descriptor.contract.hash),
            })
            .collect(),
        durabilities: vec![Durability::from(recipe.durability); recipe.components.len() - 1],
        output,
        wait: None,
        effect: None,
    };
    write(&name, draft, config)
}

fn resolve_components(
    recipe: &Recipe,
    catalog: &[ComponentEntry],
    config: Config,
) -> Result<Vec<(ComponentEntry, kairo_runtime::ComponentDescriptor)>, NewError> {
    let mut selected = Vec::with_capacity(recipe.components.len());
    let mut previous = None;
    for required in &recipe.components {
        let entry = catalog
            .iter()
            .find(|entry| entry.name == required.name)
            .cloned()
            .ok_or_else(|| NewError::MissingRecipeComponent {
                recipe: recipe.name.clone(),
                component: required.name.clone(),
            })?;
        if let Some(expected) = &required.version
            && entry.version.as_deref() != Some(expected)
        {
            return Err(NewError::RecipeVersion {
                recipe: recipe.name.clone(),
                component: required.name.clone(),
                expected: expected.clone(),
                found: entry
                    .version
                    .clone()
                    .unwrap_or_else(|| "unversioned".to_owned()),
            });
        }
        let descriptor = kairo_runtime::inspect_contract(&entry.path, config)
            .map_err(|source| NewError::Runtime { source })?;
        if !descriptor.contract.role.can_follow(previous) {
            return Err(NewError::RecipeConnection {
                recipe: recipe.name.clone(),
                previous: previous
                    .map_or("workflow input", kairo_core::ComponentRole::shape)
                    .to_owned(),
                next: descriptor.contract.shape().to_owned(),
            });
        }
        previous = Some(descriptor.contract.role);
        selected.push((entry, descriptor));
    }
    Ok(selected)
}

fn write(name: &str, draft: WorkflowDraft, config: Config) -> Result<CreatedWorkflow, NewError> {
    let source = draft.to_yaml()?;
    let workflow = Workflow::parse(&source, Path::new("."), config.max_workflow_steps)
        .map_err(|source| NewError::Workflow { source })?;
    Runtime::new(config)
        .map_err(|source| NewError::Runtime { source })?
        .validate_workflow(&workflow)
        .map_err(|source| NewError::Runtime { source })?;
    let path = PathBuf::from(format!("{name}.yaml"));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|source| NewError::Create {
            path: path.clone(),
            source,
        })?;
    file.write_all(source.as_bytes())
        .map_err(|source| NewError::Write {
            path: path.clone(),
            source,
        })?;
    println!("created and validated {}", path.display());
    Ok(CreatedWorkflow { path, run: false })
}
