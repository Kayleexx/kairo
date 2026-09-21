use std::path::Path;

use kairo_core::{Config, IoInput, IoOutput, Workflow};
use kairo_runtime::Runtime;

use crate::{Result, discovery, is_workflow, print_valid};

pub(crate) fn check(path: &Path, config: Config) -> Result<()> {
    let resolved = discovery::resolve(path, config)?;
    if is_workflow(&resolved) {
        let runtime = Runtime::new(config)?;
        let workflow = runtime.load_workflow(&resolved)?;
        runtime.validate_workflow(&workflow)?;
        print_valid(workflow_summary(&workflow));
        Ok(())
    } else {
        component(&resolved, config)
    }
}

/// shared by `kairo workflows <path>` and `kairo workflow show <path>`.
pub(crate) fn show_workflow(path: &Path, config: Config) -> Result<()> {
    let runtime = Runtime::new(config)?;
    let path = discovery::resolve(path, config)?;
    let workflow = runtime.load_workflow(&path)?;
    runtime.validate_workflow(&workflow)?;
    crate::inspection::print_workflow(&runtime, &workflow, &path);
    Ok(())
}

fn workflow_summary(workflow: &Workflow) -> String {
    let input = match workflow.io().input {
        IoInput::None => "none",
        IoInput::File => "file",
        IoInput::Value => "value",
    };
    let output = match workflow.io().output {
        IoOutput::None => "run result",
        IoOutput::Value => "value",
        IoOutput::Artifact => "artifact",
    };
    let pins = workflow
        .steps()
        .iter()
        .filter(|step| step.pinned_hash.is_some())
        .count();
    let durability = if workflow.requires_durable_artifacts() {
        "durable storage required"
    } else if workflow.has_unresolved_durability() {
        "durability auto"
    } else {
        "local durability"
    };
    let purpose = workflow
        .description()
        .map(|description| format!("\n  {description}"))
        .unwrap_or_default();
    format!(
        "workflow · {} · ready to run{purpose}\n  input · {input}\n  output · {output}\n  Components · {} ({pins} pinned · contracts verified)\n  durability · {durability}\n  next · {}",
        workflow.name(),
        workflow.steps().len(),
        run_hint(workflow)
    )
}

fn run_hint(workflow: &Workflow) -> String {
    match workflow.io().input {
        IoInput::None => format!("kairo run {}", workflow.name()),
        IoInput::File => format!("kairo run {} <file>", workflow.name()),
        IoInput::Value => format!("kairo run {} <value>", workflow.name()),
    }
}

pub(crate) fn component(path: &Path, config: Config) -> Result<()> {
    let runtime = Runtime::new(config)?;
    let loaded = runtime.load_component(path)?;
    let shape = kairo_runtime::inspect_contract(path, config)
        .map(|component| format!(" · {}", component.contract.shape()))
        .unwrap_or_default();
    print_valid(format!(
        "component · {}{shape} · {}",
        path.display(),
        loaded.hash()
    ));
    Ok(())
}
