use std::{
    fs::OpenOptions,
    io::{self, IsTerminal, Write},
    path::{Path, PathBuf},
};

use kairo_core::{Config, Durability, Workflow};
use kairo_runtime::Runtime;
use thiserror::Error;

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
    #[error("a value is required for `{field}`")]
    MissingValue { field: String },
    #[error("input must be an unsigned integer")]
    InvalidInput,
    #[error("durability must be `ephemeral` or `required`")]
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
}

pub(crate) struct CreatedWorkflow {
    pub(crate) path: PathBuf,
    pub(crate) run: bool,
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
    name: Option<String>,
    mut components: Vec<PathBuf>,
    mut input: u32,
    mut run: bool,
    config: Config,
) -> Result<CreatedWorkflow, NewError> {
    let terminal = io::stdin().is_terminal() && io::stdout().is_terminal();
    if (name.is_none() || components.is_empty()) && !terminal {
        return Err(NewError::NonInteractive);
    }
    let guided = terminal && (name.is_none() || components.is_empty());
    let name = name.map_or_else(|| prompt_required("workflow name"), Ok)?;
    valid_name(&name)?;
    let mut step_names = Vec::new();
    if components.is_empty() {
        loop {
            let path = prompt("Component path (blank when finished)", "")?;
            if path.is_empty() {
                if components.is_empty() {
                    return Err(NewError::NoSteps);
                }
                break;
            }
            let path = PathBuf::from(path);
            let default = path.file_stem().map_or_else(
                || "step".to_owned(),
                |stem| stem.to_string_lossy().into_owned(),
            );
            let step = prompt("step name", &default)?;
            components.push(path);
            step_names.push(step);
        }
    } else {
        step_names.extend(components.iter().enumerate().map(|(index, path)| {
            path.file_stem().map_or_else(
                || format!("step-{}", index + 1),
                |stem| stem.to_string_lossy().into_owned(),
            )
        }));
    }
    if name.is_empty() || components.is_empty() {
        return Err(NewError::NoSteps);
    }
    if guided && !name.is_empty() && !components.is_empty() && input == 0 {
        input = prompt("scalar input", "0")?
            .parse()
            .map_err(|_| NewError::InvalidInput)?;
    }
    let mut durabilities = Vec::new();
    for _ in 1..components.len() {
        let durability = if guided {
            prompt("edge durability (ephemeral/required)", "ephemeral")?
        } else {
            "ephemeral".to_owned()
        };
        durabilities.push(match durability.as_str() {
            "ephemeral" => Durability::Ephemeral,
            "required" => Durability::Required,
            _ => return Err(NewError::InvalidDurability),
        });
    }
    let (wait, effect) = if guided {
        let wait_kind = prompt("wait (none/timer/signal)", "none")?;
        let wait = match wait_kind.as_str() {
            "none" => None,
            "timer" => Some(format!(
                "wait:\n  timer_ms: {}\n",
                prompt("timer milliseconds", "1000")?
            )),
            "signal" => Some(format!(
                "wait:\n  signal: {}\n",
                quote(&prompt_required("signal name")?)
            )),
            _ => return Err(NewError::InvalidWait),
        };
        let effect = prompt("external action name (blank for none)", "")?;
        let effect =
            (!effect.is_empty()).then(|| format!("effect:\n  operation: {}\n", quote(&effect)));
        (wait, effect)
    } else {
        (None, None)
    };
    if guided && !run {
        run = matches!(prompt("run now? (y/n)", "y")?.as_str(), "y" | "yes");
    }
    let source = render(
        &name,
        input,
        &components,
        &step_names,
        &durabilities,
        wait,
        effect,
    );
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
    Ok(CreatedWorkflow { path, run })
}

fn render(
    name: &str,
    input: u32,
    components: &[PathBuf],
    step_names: &[String],
    durabilities: &[Durability],
    wait: Option<String>,
    effect: Option<String>,
) -> String {
    let mut source = format!("workflow: {}\ninput: {input}\n\nsteps:\n", quote(name));
    for (path, step) in components.iter().zip(step_names) {
        source.push_str(&format!(
            "  - name: {}\n    component: {}\n",
            quote(step),
            quote(&path.display().to_string())
        ));
    }
    source.push_str("\nedges:\n");
    for index in 1..components.len() {
        source.push_str(&format!(
            "  - from: {}\n    to: {}\n    durability: {}\n",
            quote(&step_names[index - 1]),
            quote(&step_names[index]),
            match durabilities[index - 1] {
                Durability::Ephemeral => "ephemeral",
                Durability::Required => "required",
            }
        ));
    }
    if let Some(wait) = wait {
        source.push('\n');
        source.push_str(&wait);
    }
    if let Some(effect) = effect {
        source.push('\n');
        source.push_str(&effect);
    }
    source
}

fn prompt_required(label: &str) -> Result<String, NewError> {
    let value = prompt(label, "")?;
    if value.is_empty() {
        Err(NewError::MissingValue {
            field: label.to_owned(),
        })
    } else {
        Ok(value)
    }
}

fn prompt(label: &str, default: &str) -> Result<String, NewError> {
    print!(
        "{label}{}: ",
        if default.is_empty() {
            "".to_owned()
        } else {
            format!(" [{default}]")
        }
    );
    io::stdout().flush().map_err(|source| NewError::Write {
        path: PathBuf::from("<prompt>"),
        source,
    })?;
    let mut value = String::new();
    io::stdin()
        .read_line(&mut value)
        .map_err(|source| NewError::Write {
            path: PathBuf::from("<prompt>"),
            source,
        })?;
    let value = value.trim().to_owned();
    Ok(if value.is_empty() {
        default.to_owned()
    } else {
        value
    })
}

fn quote(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}
