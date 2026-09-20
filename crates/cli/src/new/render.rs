use std::path::{Path, PathBuf};

use kairo_core::{
    ComponentHash, Config, DraftStep, DraftWait, Durability, WorkflowDraft, WorkflowMode,
};
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
    wait: Option<DraftWait>,
    effect: Option<String>,
) -> Result<String, NewError> {
    WorkflowDraft {
        name: name.to_owned(),
        description: None,
        accepts: Vec::new(),
        produces: Vec::new(),
        mode,
        scalar_input: input,
        steps: components
            .iter()
            .zip(hashes)
            .zip(step_names)
            .map(|((component, hash), name)| DraftStep {
                name: name.clone(),
                component: component.clone(),
                hash: *hash,
            })
            .collect(),
        durabilities: durabilities.to_vec(),
        output,
        wait,
        effect,
    }
    .to_yaml()
    .map_err(NewError::Authoring)
}
