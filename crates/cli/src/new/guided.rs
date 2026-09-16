use std::{
    fs::OpenOptions,
    io::{self, IsTerminal, Write as _},
    path::PathBuf,
};

use kairo_core::{Config, Durability, Workflow, WorkflowMode};
use kairo_runtime::Runtime;

use super::{
    CreatedWorkflow, NewError, components, default_step_name, prompt_valid_name, render, valid_name,
};

/// `kairo new`: one concept at a time, no paths, no YAML, no flags. Every real step (scaffold,
/// build, validate) reuses the exact same functions the scriptable `kairo workflow new`/`create`
/// commands already use -- this is a different front end onto the same machinery, not a second
/// authoring path.
pub(super) fn run(config: Config) -> Result<CreatedWorkflow, NewError> {
    if !(io::stdin().is_terminal() && io::stdout().is_terminal()) {
        return Err(NewError::GuidedNonInteractive);
    }
    println!("Create workflow\n");
    let name = prompt_valid_name("name", "")?;
    valid_name(&name)?;

    let mut paths: Vec<PathBuf> = Vec::new();
    let mut step_names: Vec<String> = Vec::new();
    let mut mode: Option<WorkflowMode> = None;

    loop {
        if paths.is_empty() {
            println!("\nNo steps yet.");
        }
        let (path, step_name) = add_step(&mut mode, config, step_names.len())?;
        paths.push(path);
        step_names.push(step_name);

        let again = crate::prompt::ask("\nadd another?", "no")?;
        if !is_yes(&again) {
            break;
        }
    }

    println!("\nworkflow\n  {}\n", step_names.join(" -> "));

    let mode = mode.unwrap_or(WorkflowMode::Value);
    let durabilities = vec![Durability::Auto; paths.len().saturating_sub(1)];
    let source = render::render(
        &name,
        mode,
        0,
        &paths,
        &step_names,
        &durabilities,
        None,
        None,
    );
    let workflow = Workflow::parse(
        &source,
        std::path::Path::new("."),
        config.max_workflow_steps,
    )
    .map_err(|source| NewError::Workflow { source })?;
    let runtime = Runtime::new(config).map_err(|source| NewError::Runtime { source })?;
    runtime
        .validate_workflow(&workflow)
        .map_err(|source| NewError::Runtime { source })?;

    let out = PathBuf::from(format!("{name}.yaml"));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&out)
        .map_err(|source| NewError::Create {
            path: out.clone(),
            source,
        })?;
    file.write_all(source.as_bytes())
        .map_err(|source| NewError::Write {
            path: out.clone(),
            source,
        })?;

    println!("✓ {name} created");
    Ok(CreatedWorkflow {
        path: out,
        run: false,
    })
}

/// loops until one compatible step is chosen and added -- never returns a step that would break
/// the workflow's established interface, and explains why one was rejected instead of just
/// failing later at run time.
fn add_step(
    mode: &mut Option<WorkflowMode>,
    config: Config,
    step_index: usize,
) -> Result<(PathBuf, String), NewError> {
    loop {
        println!("\nWhat do you want to do?");
        println!("  1. create a new component");
        println!("  2. use existing component");
        println!("  3. import component");
        let choice = crate::prompt::ask("choose", "1")?;
        let picked = match choice.as_str() {
            "1" => create_new(step_index)?,
            "2" => pick_existing(*mode, config, step_index)?,
            "3" => import(step_index)?,
            _ => {
                println!("error: choose 1, 2, or 3");
                continue;
            }
        };
        let Some((path, step_name)) = picked else {
            continue;
        };
        match render::detect_mode(&path, config) {
            Ok(detected) if mode.is_none_or(|established| established == detected) => {
                *mode = Some(detected);
                return Ok((path, step_name));
            }
            Ok(_) => println!(
                "error: this component does not fit the rest of the workflow, try a different one"
            ),
            Err(error) => println!("error: {error}"),
        }
    }
}

fn create_new(step_index: usize) -> Result<Option<(PathBuf, String)>, NewError> {
    let step = prompt_valid_name("step name", &format!("step-{}", step_index + 1))?;
    match components::resolve_named_component(&step) {
        Ok(path) => {
            println!("✓ scaffolded {step}");
            Ok(Some((path, step)))
        }
        Err(error) => {
            println!("error: {error}");
            Ok(None)
        }
    }
}

fn pick_existing(
    established: Option<WorkflowMode>,
    config: Config,
    step_index: usize,
) -> Result<Option<(PathBuf, String)>, NewError> {
    let candidates: Vec<PathBuf> = components::list_components()
        .into_iter()
        .filter(|path| {
            render::detect_mode(path, config)
                .is_ok_and(|detected| established.is_none_or(|mode| mode == detected))
        })
        .collect();
    if candidates.is_empty() {
        println!("no existing components fit here yet");
        return Ok(None);
    }
    println!("existing components");
    for (index, path) in candidates.iter().enumerate() {
        println!("  {} · {}", index + 1, components::component_label(path));
    }
    let choice = crate::prompt::ask("pick a number, or leave blank to go back", "")?;
    if choice.is_empty() {
        return Ok(None);
    }
    let Some(path) = choice
        .parse::<usize>()
        .ok()
        .and_then(|index| candidates.get(index.saturating_sub(1)).cloned())
    else {
        println!("error: not a valid choice");
        return Ok(None);
    };
    let default = default_step_name(&path, step_index);
    let step = prompt_valid_name("step name", &default)?;
    Ok(Some((path, step)))
}

fn import(step_index: usize) -> Result<Option<(PathBuf, String)>, NewError> {
    let raw = crate::prompt::required("component path")?;
    let path = PathBuf::from(&raw);
    if !path.is_file() {
        println!("error: `{raw}` was not found");
        return Ok(None);
    }
    let default = default_step_name(&path, step_index);
    let step = prompt_valid_name("step name", &default)?;
    Ok(Some((path, step)))
}

fn is_yes(value: &str) -> bool {
    matches!(value.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}
