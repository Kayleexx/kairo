use std::{
    fs::OpenOptions,
    io::{self, IsTerminal, Write as _},
    path::PathBuf,
};

use kairo_core::{ComponentHash, Config, Durability, Workflow, WorkflowMode};
use kairo_runtime::{ComponentRole, Runtime};

use super::{CreatedWorkflow, NewError, prompt_valid_name, render, select, valid_name};

pub(super) fn run(name: Option<String>, config: Config) -> Result<CreatedWorkflow, NewError> {
    if !(io::stdin().is_terminal() && io::stdout().is_terminal()) {
        return Err(NewError::GuidedNonInteractive);
    }
    println!("Create workflow\n");
    let name = name.map_or_else(|| prompt_valid_name("name", ""), Ok)?;
    valid_name(&name)?;

    let catalog = kairo_runtime::catalog_components(config);
    let selected = if catalog.is_empty() {
        select::select_by_import(config)?
    } else {
        select::select_from_catalog(&catalog, config)?
    };

    println!(
        "\nKairo found one valid typed chain:\n  {}\n",
        select::chain_label(&selected)
    );
    if !select::ask_yes_no("Create workflow?", true)? {
        return Err(NewError::Cancelled);
    }

    let paths: Vec<PathBuf> = selected.iter().map(|item| item.path.clone()).collect();
    let hashes: Vec<Option<ComponentHash>> = selected.iter().map(|item| item.hash).collect();
    let step_names: Vec<String> = selected.iter().map(|item| item.name.clone()).collect();
    // the loop above always selects at least one Component before returning, so this is safe.
    let role = selected
        .last()
        .map_or(ComponentRole::ValueStage, |item| item.role);
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
        WorkflowMode::Value => format!("next · kairo run {name} <input>"),
        WorkflowMode::Stream => format!("next · kairo run {name} <input-file>"),
        WorkflowMode::Scalar => format!("next · kairo run {name}"),
    }
}

fn prompt_output_artifact(name: &str) -> Result<(String, String), NewError> {
    let filename = crate::prompt::ask("output filename", &format!("{name}.bin"))?;
    let content_type = crate::prompt::ask("output content type", "application/octet-stream")?;
    Ok((filename, content_type))
}
