use kairo_core::{ComponentRole, Config};

pub use kairo_runtime::CatalogComponent;

pub fn catalog(config: Config) -> Vec<CatalogComponent> {
    kairo_runtime::catalog_components(config)
}

pub fn compatible(
    components: &[CatalogComponent],
    previous: Option<ComponentRole>,
) -> Vec<&CatalogComponent> {
    kairo_runtime::compatible_components(components, previous)
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
    role.finishable(step_count)
}
