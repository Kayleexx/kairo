use std::path::{Path, PathBuf};

use kairo_core::{ComponentHash, Config, Durability, WorkflowMode};
use kairo_runtime::ComponentContract;

use super::NewError;

pub(super) fn detect_contract(
    component: &Path,
    config: Config,
) -> Result<ComponentContract, NewError> {
    kairo_runtime::detect_contract(component, config).ok_or_else(|| {
        NewError::UnsupportedComponent {
            path: component.to_path_buf(),
        }
    })
}

pub(super) fn detect_mode(component: &Path, config: Config) -> Result<WorkflowMode, NewError> {
    detect_contract(component, config).map(|contract| contract.mode())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn render(
    name: &str,
    mode: WorkflowMode,
    input: u32,
    components: &[PathBuf],
    hashes: &[Option<ComponentHash>],
    step_names: &[String],
    durabilities: &[Durability],
    output: Option<(String, String)>,
    wait: Option<String>,
    effect: Option<String>,
) -> String {
    kairo_tui::compose::render(
        name,
        mode,
        input,
        components,
        hashes,
        step_names,
        durabilities,
        output,
        wait,
        effect,
    )
}
