use std::{
    collections::HashSet,
    fs::{self, OpenOptions},
    io::{self, IsTerminal, Write},
    path::{Path, PathBuf},
};

use kairo_core::{Config, Durability, Workflow, WorkflowMode};
use kairo_runtime::Runtime;
use thiserror::Error;

mod render;

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
    #[error("at least one Component is required")]
    NoSteps,
    #[error("Component `{path}` was not found")]
    MissingComponent { path: PathBuf },
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
    #[error(
        "Component `{path}` implements neither the scalar nor the value workflow interface; \
         scaffold one with `kairo component new`"
    )]
    UnsupportedComponent { path: PathBuf },
}

pub(crate) struct CreatedWorkflow {
    pub(crate) path: PathBuf,
    pub(crate) run: bool,
}

pub(crate) struct CreateOptions {
    pub(crate) name: Option<String>,
    pub(crate) components: Vec<PathBuf>,
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
        name,
        mut components,
        mut input,
        run,
        durability,
        wait,
        effect,
        advanced,
    } = options;
    let terminal = io::stdin().is_terminal() && io::stdout().is_terminal();
    if (name.is_none() || components.is_empty()) && !terminal {
        return Err(NewError::NonInteractive);
    }
    let guided = terminal && (name.is_none() || components.is_empty());
    let name = name.map_or_else(|| prompt_valid_name("workflow name", ""), Ok)?;
    valid_name(&name)?;
    let mut step_names = Vec::new();
    if components.is_empty() {
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
            let default = path.file_stem().map_or_else(
                || "step".to_owned(),
                |stem| stem.to_string_lossy().into_owned(),
            );
            let step = prompt_valid_name("step name", &default)?;
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
        (parse_wait_option(wait)?, parse_effect_option(effect)?)
    } else if guided && advanced {
        let wait_kind = crate::prompt::ask("wait (none/timer/signal)", "none")?;
        let wait = match wait_kind.as_str() {
            "none" => None,
            "timer" => Some(format!(
                "wait:\n  timer_ms: {}\n",
                crate::prompt::ask("timer milliseconds", "1000")?
            )),
            "signal" => Some(format!(
                "wait:\n  signal: {}\n",
                crate::prompt::quote(&crate::prompt::required("signal name")?)
            )),
            _ => return Err(NewError::InvalidWait),
        };
        let effect = crate::prompt::ask("external action name (blank for none)", "")?;
        let effect = (!effect.is_empty())
            .then(|| format!("effect:\n  operation: {}\n", crate::prompt::quote(&effect)));
        (wait, effect)
    } else {
        (None, None)
    };
    let source = render::render(
        &name,
        mode,
        input,
        &components,
        &step_names,
        &durabilities,
        wait,
        effect,
    );
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
fn default_step_names(components: &[PathBuf]) -> Vec<String> {
    let mut used = HashSet::new();
    components
        .iter()
        .enumerate()
        .map(|(index, path)| {
            let base = default_step_name(path, index);
            let mut name = base.clone();
            let mut suffix = 2;
            while used.contains(&name) {
                name = format!("{base}-{suffix}");
                suffix += 1;
            }
            used.insert(name.clone());
            name
        })
        .collect()
}

fn default_step_name(path: &Path, index: usize) -> String {
    match path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
    {
        Some(stem) if stem != "component" && !stem.is_empty() => stem,
        _ => path.parent().and_then(Path::file_name).map_or_else(
            || format!("step-{}", index + 1),
            |name| name.to_string_lossy().into_owned(),
        ),
    }
}

fn choose_component(value: &str) -> Result<PathBuf, NewError> {
    let discovered = discover_components();
    let path = value
        .parse::<usize>()
        .ok()
        .and_then(|index| discovered.get(index.saturating_sub(1)).cloned())
        .unwrap_or_else(|| PathBuf::from(value));
    if !path.is_file() {
        return Err(NewError::MissingComponent { path });
    }
    Ok(path)
}

fn discover_components() -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir("components") else {
        return Vec::new();
    };
    let mut paths: Vec<_> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && matches!(
                    path.extension().and_then(|extension| extension.to_str()),
                    Some("wasm" | "wat" | "wast")
                )
        })
        .collect();
    paths.sort();
    if !paths.is_empty() {
        println!("available Components");
        for (index, path) in paths.iter().enumerate() {
            println!("  {} · {}", index + 1, path.display());
        }
    }
    paths
}

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

fn parse_wait_option(value: Option<String>) -> Result<Option<String>, NewError> {
    let Some(value) = value else { return Ok(None) };
    if let Some(milliseconds) = value.strip_prefix("timer:") {
        let milliseconds = milliseconds
            .parse::<u64>()
            .map_err(|_| NewError::InvalidWait)?;
        return Ok(Some(format!("wait:\n  timer_ms: {milliseconds}\n")));
    }
    if let Some(signal) = value.strip_prefix("signal:") {
        if signal.is_empty() || signal.chars().any(char::is_control) {
            return Err(NewError::InvalidWait);
        }
        return Ok(Some(format!(
            "wait:\n  signal: {}\n",
            crate::prompt::quote(signal)
        )));
    }
    Err(NewError::InvalidWait)
}

fn parse_effect_option(value: Option<String>) -> Result<Option<String>, NewError> {
    let Some(value) = value else { return Ok(None) };
    if value.is_empty()
        || value.len() > 64
        || !value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        return Err(NewError::MissingValue {
            field: "effect operation".to_owned(),
        });
    }
    Ok(Some(format!(
        "effect:\n  operation: {}\n",
        crate::prompt::quote(&value)
    )))
}
