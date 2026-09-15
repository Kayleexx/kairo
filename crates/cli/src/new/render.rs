use std::path::{Path, PathBuf};

use kairo_core::{Config, Durability, Workflow, WorkflowMode};
use kairo_runtime::Runtime;

use super::NewError;

/// tries a component against each mode's real interface (a throwaway single-step workflow,
/// validated the same way any workflow is) rather than inspecting its WIT directly -- reuses the
/// one check `kairo run`/`kairo check` already trust, instead of a second, parallel notion of
/// "what interface does this component implement".
pub(super) fn detect_mode(component: &Path, config: Config) -> Result<WorkflowMode, NewError> {
    let path = crate::prompt::quote(&component.display().to_string());
    let value_source = format!(
        "workflow: probe\nmode: value\nsteps:\n  - name: probe\n    component: {path}\nedges: []\n"
    );
    if validates(&value_source, config) {
        return Ok(WorkflowMode::Value);
    }
    let scalar_source = format!(
        "workflow: probe\ninput: 0\nsteps:\n  - name: probe\n    component: {path}\nedges: []\n"
    );
    if validates(&scalar_source, config) {
        return Ok(WorkflowMode::Scalar);
    }
    Err(NewError::UnsupportedComponent {
        path: component.to_path_buf(),
    })
}

fn validates(source: &str, config: Config) -> bool {
    let Ok(workflow) = Workflow::parse(source, Path::new("."), config.max_workflow_steps) else {
        return false;
    };
    let Ok(runtime) = Runtime::new(config) else {
        return false;
    };
    runtime.validate_workflow(&workflow).is_ok()
}

#[allow(clippy::too_many_arguments)]
pub(super) fn render(
    name: &str,
    mode: WorkflowMode,
    input: u32,
    components: &[PathBuf],
    step_names: &[String],
    durabilities: &[Durability],
    wait: Option<String>,
    effect: Option<String>,
) -> String {
    let mut source = format!("workflow: {}\n", crate::prompt::quote(name));
    if mode == WorkflowMode::Value {
        source.push_str("mode: value\nio:\n  input: value\n  output: value\n\nsteps:\n");
    } else {
        source.push_str(&format!("input: {input}\n\nsteps:\n"));
    }
    for (path, step) in components.iter().zip(step_names) {
        source.push_str(&format!(
            "  - name: {}\n    component: {}\n",
            crate::prompt::quote(step),
            crate::prompt::quote(&path.display().to_string())
        ));
    }
    source.push_str("\nedges:\n");
    for index in 1..components.len() {
        source.push_str(&format!(
            "  - from: {}\n    to: {}\n    durability: {}\n",
            crate::prompt::quote(&step_names[index - 1]),
            crate::prompt::quote(&step_names[index]),
            match durabilities[index - 1] {
                Durability::Ephemeral => "ephemeral",
                Durability::Required => "required",
                Durability::Auto => "auto",
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
