use std::path::{Path, PathBuf};

use kairo_core::{Config, Durability, WorkflowMode, catalog::ComponentEntry};
use kairo_runtime::{ComponentContract, ComponentRole, detect_contract};

#[derive(Clone)]
pub struct CatalogComponent {
    pub entry: ComponentEntry,
    pub contract: ComponentContract,
}

pub fn catalog(config: Config) -> Vec<CatalogComponent> {
    kairo_core::catalog::list(&[Path::new("components"), Path::new("components/reference")])
        .into_iter()
        .filter_map(|entry| {
            let contract = detect_contract(&entry.path, config)?;
            Some(CatalogComponent { entry, contract })
        })
        .collect()
}

pub fn compatible(
    components: &[CatalogComponent],
    previous: Option<ComponentRole>,
) -> Vec<&CatalogComponent> {
    components
        .iter()
        .filter(|component| component.contract.can_follow(previous))
        .collect()
}

pub fn describe(component: &CatalogComponent) -> String {
    match &component.entry.description {
        Some(description) => format!(
            "{} · {description} · {}",
            component.entry.name,
            component.contract.shape()
        ),
        None => format!("{} · {}", component.entry.name, component.contract.shape()),
    }
}

/// a stream chain can't legally end on a `StreamTransform` step, and can't end on a single
/// terminal step with no declared artifact output either (`WorkflowError::StreamWorkflowSteps`
/// requires >=2 steps unless `output:` is set).
pub fn finishable(role: ComponentRole, step_count: usize) -> bool {
    role.allows_finish()
        && (role.mode() != WorkflowMode::Stream
            || role == ComponentRole::StreamOutput
            || step_count >= 2)
}

#[allow(clippy::too_many_arguments)]
pub fn render(
    name: &str,
    mode: WorkflowMode,
    input: u32,
    components: &[PathBuf],
    step_names: &[String],
    durabilities: &[Durability],
    output: Option<(String, String)>,
    wait: Option<String>,
    effect: Option<String>,
) -> String {
    let mut source = format!("workflow: {}\n", quote(name));
    match mode {
        WorkflowMode::Value => {
            source.push_str("mode: value\nio:\n  input: value\n  output: value\n\nsteps:\n");
        }
        WorkflowMode::Stream => {
            source.push_str("mode: stream\n");
            if let Some((filename, content_type)) = output {
                source.push_str(&format!(
                    "output:\n  filename: {}\n  content_type: {}\n",
                    quote(&filename),
                    quote(&content_type)
                ));
            }
            source.push_str("\nsteps:\n");
        }
        WorkflowMode::Scalar => {
            source.push_str(&format!("input: {input}\n\nsteps:\n"));
        }
    }
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

fn quote(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}
