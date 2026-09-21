use std::{
    fs::OpenOptions,
    io::{self, IsTerminal, Write},
    path::{Path, PathBuf},
};

use kairo_core::{Config, DraftWait, Durability, Workflow, WorkflowMode};
use kairo_runtime::Runtime;
use thiserror::Error;

use components::{
    choose_component, has_available_components, print_no_components_guidance,
    resolve_named_component,
};

mod components;
mod guided;
mod options;
mod recipe;
mod render;
mod select;

use options::{default_step_name, default_step_names, parse_effect, parse_wait};

pub(crate) fn guided(name: Option<String>, config: Config) -> Result<CreatedWorkflow, NewError> {
    guided::run(name, config)
}

pub(crate) fn from_recipe(
    name: Option<String>,
    recipe_name: &str,
    config: Config,
) -> Result<CreatedWorkflow, NewError> {
    recipe::from_recipe(name, recipe_name, config)
}

pub(crate) fn list_recipes(config: Config) {
    recipe::list(config);
}

#[derive(Debug, Error)]
pub(crate) enum NewError {
    #[error("invalid workflow name `{name}`; use 1-64 letters, numbers, `_`, or `-`")]
    InvalidName { name: String },
    #[error("component path must be valid UTF-8 and contain no control characters")]
    InvalidComponentPath,
    #[error("failed to create workflow `{path}`")]
    Create {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to write workflow `{path}`")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("workflow creation needs an interactive terminal when name or components are omitted")]
    NonInteractive,
    #[error(
        "`kairo new` needs an interactive terminal.\n\nfor scripts, use:\n  kairo workflow new <name> <component-names...>\n  kairo workflow create --name <name> --component <path> ..."
    )]
    GuidedNonInteractive,
    #[error("at least one Component is required")]
    NoSteps,
    #[error(
        "Component `{name}` is not registered\n\nadd it with:\n  kairo add <path> --name {name}"
    )]
    UnknownComponent { name: String },
    #[error("a value is required for `{field}`")]
    MissingValue { field: String },
    #[error("input must be an unsigned integer")]
    InvalidInput,
    #[error("durability must be `ephemeral`, `required`, or `auto`")]
    InvalidDurability,
    #[error("wait must be `none`, `timer`, or `signal`")]
    InvalidWait,
    #[error("workflow validation failed")]
    Workflow {
        #[source]
        source: kairo_core::WorkflowError,
    },
    #[error("workflow Component validation failed")]
    Runtime {
        #[source]
        source: kairo_runtime::RuntimeError,
    },
    #[error(transparent)]
    Authoring(#[from] kairo_core::AuthoringError),
    #[error(
        "Component `{path}` implements neither the scalar nor the value workflow interface; \
         scaffold one with `kairo component new`"
    )]
    UnsupportedComponent { path: PathBuf },
    #[error(transparent)]
    Component(#[from] crate::component::ComponentError),
    #[error(transparent)]
    Recipe(#[from] kairo_core::recipe::RecipeError),
    #[error("recipe `{recipe}` was not found in `recipes/`")]
    MissingRecipe { recipe: String },
    #[error(
        "recipe `{recipe}` needs Components that are not registered:\n{components}\n\nadd them, then retry `kairo new --recipe {recipe}`"
    )]
    MissingRecipeComponents { recipe: String, components: String },
    #[error("recipe `{recipe}` requires {component} {expected}, but {found} is registered")]
    RecipeVersion {
        recipe: String,
        component: String,
        expected: String,
        found: String,
    },
    #[error("recipe `{recipe}` has incompatible Components: {previous} cannot connect to {next}")]
    RecipeConnection {
        recipe: String,
        previous: String,
        next: String,
    },
    #[error("Components are incompatible: `{previous}` cannot connect to `{next}`")]
    ComponentConnection { previous: String, next: String },
    #[error("cancelled")]
    Cancelled,
}

pub(crate) struct CreatedWorkflow {
    pub(crate) path: PathBuf,
    pub(crate) run: bool,
}

pub(crate) struct CreateOptions {
    pub(crate) name: Option<String>,
    pub(crate) components: Vec<PathBuf>,
    pub(crate) steps: Vec<String>,
    pub(crate) input: u32,
    pub(crate) run: bool,
    pub(crate) durability: Option<String>,
    pub(crate) wait: Option<String>,
    pub(crate) effect: Option<String>,
    pub(crate) advanced: bool,
}

pub(crate) fn workflow(
    name: &str,
    component: &std::path::Path,
    input: u32,
) -> Result<(), NewError> {
    valid_name(name)?;
    let component = component
        .to_str()
        .filter(|path| !path.chars().any(char::is_control))
        .ok_or(NewError::InvalidComponentPath)?
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    let path = PathBuf::from(format!("{name}.yaml"));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|source| NewError::Create {
            path: path.clone(),
            source,
        })?;
    write!(
        file,
        "workflow: {name}\ninput: {input}\n\nsteps:\n  - name: run\n    component: \"{component}\"\n\nedges: []\n"
    )
    .map_err(|source| NewError::Write {
        path: path.clone(),
        source,
    })?;
    println!(
        "created {}\nnext · kairo run {}",
        path.display(),
        path.display()
    );
    Ok(())
}

fn valid_name(name: &str) -> Result<(), NewError> {
    if name.is_empty()
        || name.len() > 64
        || !name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        return Err(NewError::InvalidName {
            name: name.to_owned(),
        });
    }
    Ok(())
}

pub(crate) fn interactive(
    options: CreateOptions,
    config: Config,
) -> Result<CreatedWorkflow, NewError> {
    let CreateOptions {
        mut name,
        mut components,
        steps,
        mut input,
        run,
        durability,
        wait,
        effect,
        advanced,
    } = options;
    components = components
        .into_iter()
        .map(|path| {
            if path.is_file() {
                Ok(path)
            } else {
                resolve_named_component(&path.to_string_lossy())
            }
        })
        .collect::<Result<_, NewError>>()?;
    let mut steps = steps.into_iter();
    if name.is_none() {
        name = steps.next();
    }
    for step in steps {
        components.push(resolve_named_component(&step)?);
    }
    let terminal = io::stdin().is_terminal() && io::stdout().is_terminal();
    if (name.is_none() || components.is_empty()) && !terminal {
        return Err(NewError::NonInteractive);
    }
    let guided = terminal && (name.is_none() || components.is_empty());
    let name = name.map_or_else(|| prompt_valid_name("workflow name", ""), Ok)?;
    valid_name(&name)?;
    let mut step_names = Vec::new();
    if components.is_empty() {
        if !has_available_components() {
            print_no_components_guidance();
        }
        loop {
            let path = crate::prompt::ask("Component path (blank when finished)", "")?;
            if path.is_empty() {
                if components.is_empty() {
                    return Err(NewError::NoSteps);
                }
                break;
            }
            let path = match choose_component(&path) {
                Ok(path) => path,
                Err(error) => {
                    println!("error: {error}");
                    continue;
                }
            };
            // the component's own name is already a good, collision-safe step name (the same
            // logic the non-interactive path already uses) -- no need to ask for one too.
            let mut step = default_step_name(&path, step_names.len());
            let mut suffix = 2;
            while step_names.contains(&step) {
                step = format!("{step}-{suffix}");
                suffix += 1;
            }
            components.push(path);
            step_names.push(step);
        }
    } else {
        step_names.extend(default_step_names(&components));
    }
    if name.is_empty() || components.is_empty() {
        return Err(NewError::NoSteps);
    }
    // the first Component's real interface decides the workflow's mode -- a mismatched later
    // step still fails loudly, just later, at the same `validate_workflow` call every mode uses.
    let mode = render::detect_mode(&components[0], config)?;
    select::check_connections(&components, config)?;
    // pin every step's real content hash where it's detectable, so a component that changes
    // after this workflow was composed is refused at run time instead of silently swapped in;
    // best-effort here on purpose -- a component that fails detection still fails loudly and
    // clearly at `validate_workflow` below, this just skips pinning for it.
    let hashes: Vec<Option<kairo_core::ComponentHash>> = components
        .iter()
        .map(|component| {
            render::detect_contract(component, config)
                .ok()
                .map(|contract| contract.hash)
        })
        .collect();
    if guided && mode == WorkflowMode::Scalar && !components.is_empty() && input == 0 {
        input = crate::prompt::ask("scalar input", "0")?
            .parse()
            .map_err(|_| NewError::InvalidInput)?;
    }
    let mut durabilities = Vec::new();
    for _ in 1..components.len() {
        // default to `auto` so a first-time author never has to understand durability cuts
        // themselves -- the planner measures the real tradeoff once profiled.
        let durability = if let Some(value) = durability.as_deref() {
            value.to_owned()
        } else if guided && advanced {
            crate::prompt::ask("edge durability (ephemeral/required/auto)", "auto")?
        } else {
            "auto".to_owned()
        };
        durabilities.push(match durability.as_str() {
            "ephemeral" => Durability::Ephemeral,
            "required" => Durability::Required,
            "auto" => Durability::Auto,
            _ => return Err(NewError::InvalidDurability),
        });
    }
    let (wait, effect) = if wait.is_some() || effect.is_some() {
        (parse_wait(wait)?, parse_effect(effect)?)
    } else if guided && advanced {
        let wait_kind = crate::prompt::ask("wait (none/timer/signal)", "none")?;
        let wait = match wait_kind.as_str() {
            "none" => None,
            "timer" => Some(DraftWait::Timer(
                crate::prompt::ask("timer milliseconds", "1000")?
                    .parse()
                    .map_err(|_| NewError::InvalidWait)?,
            )),
            "signal" => Some(DraftWait::Signal(crate::prompt::required("signal name")?)),
            _ => return Err(NewError::InvalidWait),
        };
        let effect = crate::prompt::ask("external action name (blank for none)", "")?;
        let effect = (!effect.is_empty()).then_some(effect);
        (wait, effect)
    } else {
        (None, None)
    };
    let source = render::render(
        &name,
        mode,
        input,
        &components,
        &hashes,
        &step_names,
        &durabilities,
        None,
        wait,
        effect,
    )?;
    let workflow = Workflow::parse(&source, Path::new("."), config.max_workflow_steps)
        .map_err(|source| NewError::Workflow { source })?;
    let runtime = Runtime::new(config).map_err(|source| NewError::Runtime { source })?;
    runtime
        .validate_workflow(&workflow)
        .map_err(|source| NewError::Runtime { source })?;
    let path = PathBuf::from(format!("{name}.yaml"));
    println!("\npreview");
    crate::inspection::print_workflow(&runtime, &workflow, &path);
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
    Ok(CreatedWorkflow { path, run })
}

/// every component `kairo component build` produces is named `component.wasm`, so the file stem
/// alone collides for any two-step pipeline built the standard way -- fall back to the project
/// directory name in that case, then number any name still left colliding.
fn prompt_valid_name(label: &str, default: &str) -> Result<String, NewError> {
    loop {
        let value = crate::prompt::ask(label, default)?;
        if value.is_empty() && default.is_empty() {
            return Err(NewError::MissingValue {
                field: label.to_owned(),
            });
        }
        match valid_name(&value) {
            Ok(()) => return Ok(value),
            Err(error) => println!("error: {error}"),
        }
    }
}
