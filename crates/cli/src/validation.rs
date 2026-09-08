use std::path::Path;

use kairo_core::Config;
use kairo_runtime::Runtime;

use crate::{Result, is_workflow, print_valid};

pub(crate) fn check(path: &Path, config: Config) -> Result<()> {
    if is_workflow(path) {
        let runtime = Runtime::new(config)?;
        let workflow = runtime.load_workflow(path)?;
        runtime.validate_workflow(&workflow)?;
        print_valid(format!(
            "workflow · {} · {} components",
            path.display(),
            workflow.steps().len()
        ));
        Ok(())
    } else {
        component(path, config)
    }
}

pub(crate) fn component(path: &Path, config: Config) -> Result<()> {
    let component = Runtime::new(config)?.load_component(path)?;
    print_valid(format!(
        "component · {} · {}",
        path.display(),
        component.hash()
    ));
    Ok(())
}
