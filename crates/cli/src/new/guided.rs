use std::{
    collections::HashSet,
    fs::OpenOptions,
    io::{self, IsTerminal, Write as _},
    path::PathBuf,
};

use kairo_core::{ComponentHash, Config, Durability, Workflow, WorkflowMode};
use kairo_runtime::{ComponentRole, Runtime};

use super::{CreatedWorkflow, NewError, default_step_name, prompt_valid_name, render, valid_name};

pub(super) fn run(name: Option<String>, config: Config) -> Result<CreatedWorkflow, NewError> {
    if !(io::stdin().is_terminal() && io::stdout().is_terminal()) {
        return Err(NewError::GuidedNonInteractive);
    }
    println!("Create workflow\n");
    let name = name.map_or_else(|| prompt_valid_name("name", ""), Ok)?;
    valid_name(&name)?;

    let mut paths: Vec<PathBuf> = Vec::new();
    let mut hashes: Vec<Option<ComponentHash>> = Vec::new();
    let mut step_names: Vec<String> = Vec::new();
    let mut role: Option<ComponentRole> = None;

    loop {
        let (path, hash, step_name, picked_role) = add_step(role, config, &paths, &step_names)?;
        paths.push(path);
        hashes.push(hash);
        step_names.push(step_name);
        role = Some(picked_role);

        if picked_role.finishable(paths.len()) {
            if !picked_role.can_continue() || !ask_yes_no("\nadd another?", false)? {
                break;
            }
        } else {
            println!("\n(not finished yet -- this needs at least one more step)");
        }
    }

    println!("\nworkflow\n  {}\n", step_names.join(" -> "));

    // the loop above always adds at least one step before it can break, so `role` is always set.
    let role = role.unwrap_or(ComponentRole::ValueStage);
    let mode = role.mode();
    let output = (role == ComponentRole::StreamOutput)
        .then(|| prompt_output_artifact(&name))
        .transpose()?;
    let durabilities = vec![Durability::Auto; paths.len().saturating_sub(1)];
    let source = render::render(
        &name,
        mode,
        0,
        &paths,
        &hashes,
        &step_names,
        &durabilities,
        output,
        None,
        None,
    )?;
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
    println!("{}", run_hint(&name, mode));
    Ok(CreatedWorkflow {
        path: out,
        run: false,
    })
}

fn run_hint(name: &str, mode: WorkflowMode) -> String {
    match mode {
        WorkflowMode::Value => format!("next · kairo run {name} --value <input>"),
        WorkflowMode::Stream => format!("next · kairo run {name} <input-file>"),
        WorkflowMode::Scalar => format!("next · kairo run {name}"),
    }
}

fn prompt_output_artifact(name: &str) -> Result<(String, String), NewError> {
    let filename = crate::prompt::ask("output filename", &format!("{name}.bin"))?;
    let content_type = crate::prompt::ask("output content type", "application/octet-stream")?;
    Ok((filename, content_type))
}

fn add_step(
    role: Option<ComponentRole>,
    config: Config,
    used: &[PathBuf],
    step_names: &[String],
) -> Result<(PathBuf, Option<ComponentHash>, String, ComponentRole), NewError> {
    let step_index = step_names.len();
    let label = if step_index == 0 {
        "first Component"
    } else {
        "next Component"
    };
    loop {
        let catalog = kairo_runtime::catalog_components(config);
        let candidates = kairo_runtime::compatible_components(&catalog, role);
        if !candidates.is_empty() {
            println!("compatible Components");
            for component in &candidates {
                let marker = if used.contains(&component.entry.path) {
                    " · already used"
                } else {
                    ""
                };
                let description = component.entry.description.as_deref().map_or_else(
                    || format!("{} · {}", component.entry.name, component.contract.shape()),
                    |description| {
                        format!(
                            "{} · {description} · {}",
                            component.entry.name,
                            component.contract.shape()
                        )
                    },
                );
                println!("  {description}{marker}");
            }
        }
        let prompt = if candidates.is_empty() {
            label.to_owned()
        } else {
            format!("{label} (type a name above, or \"import\")")
        };
        let value = crate::prompt::ask(&prompt, "")?;
        if let Some(component) = candidates
            .iter()
            .find(|component| component.entry.name == value)
        {
            let step = unique_step_name(step_names, &component.entry.name);
            return Ok((
                component.entry.path.clone(),
                Some(component.contract.hash),
                step,
                component.contract.role,
            ));
        }
        let picked = match value.as_str() {
            "import" => import(step_index)?,
            name if !name.is_empty() && crate::component::valid_name(name) => {
                unknown_name(name, step_index)?
            }
            _ => {
                println!(
                    "error: use 1-64 lowercase letters, digits, `-`, or `_`, starting with a \
                     letter (or type \"import\")"
                );
                continue;
            }
        };
        let Some((path, step_name)) = picked else {
            continue;
        };
        match render::detect_contract(&path, config) {
            Ok(contract) if contract.can_follow(role) => {
                return Ok((path, Some(contract.hash), step_name, contract.role));
            }
            Ok(_) => println!(
                "error: this component does not fit the rest of the workflow, try a different one"
            ),
            Err(error) => println!("error: {error}"),
        }
    }
}

fn unknown_name(name: &str, step_index: usize) -> Result<Option<(PathBuf, String)>, NewError> {
    println!(
        "\nno registered Component named \"{name}\" found\n  1. search again\n  2. import component"
    );
    let choice = crate::prompt::ask("choice", "1")?;
    match choice.trim() {
        "2" => import(step_index),
        _ => Ok(None),
    }
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

fn unique_step_name(step_names: &[String], base: &str) -> String {
    let used: HashSet<&str> = step_names.iter().map(String::as_str).collect();
    if !used.contains(base) {
        return base.to_owned();
    }
    let mut suffix = 2;
    loop {
        let candidate = format!("{base}-{suffix}");
        if !used.contains(candidate.as_str()) {
            return candidate;
        }
        suffix += 1;
    }
}

fn ask_yes_no(prompt: &str, default: bool) -> Result<bool, NewError> {
    loop {
        let value = crate::prompt::ask(prompt, if default { "yes" } else { "no" })?;
        match value.trim().to_ascii_lowercase().as_str() {
            "y" | "yes" => return Ok(true),
            "n" | "no" => return Ok(false),
            _ => println!("error: answer \"yes\" or \"no\""),
        }
    }
}
