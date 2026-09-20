use std::path::Path;

use kairo_core::Config;
use kairo_runtime::Runtime;

use crate::{Result, discovery, is_workflow, print_valid};

pub(crate) fn check(path: &Path, config: Config) -> Result<()> {
    let resolved = discovery::resolve(path, config)?;
    if is_workflow(&resolved) {
        let runtime = Runtime::new(config)?;
        let workflow = runtime.load_workflow(&resolved)?;
        runtime.validate_workflow(&workflow)?;
        print_valid(format!(
            "workflow · {} · ready to run · {} components",
            workflow.name(),
            workflow.steps().len()
        ));
        Ok(())
    } else {
        component(&resolved, config)
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
