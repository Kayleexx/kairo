use std::path::Path;

use super::select_cell;

/// reruns a durable run against its own recorded state file -- exactly what a user already
/// achieves by manually repeating `kairo run <workflow> --run <same-name>`, just under an
/// explicit, discoverable verb.
pub(crate) async fn resume(
    run: &str,
    config: kairo_core::Config,
    verbose: bool,
) -> crate::Result<()> {
    let cell = select_cell(Some(Path::new(run)))?;
    let name = match super::inspect_value(&cell.path)? {
        Some(inspection) => inspection.name,
        None => super::inspect(&cell.path)?.name,
    };
    let workflow_name = name.unwrap_or(cell.name);
    crate::execution::run_path(
        Path::new(&workflow_name),
        crate::execution::RunOptions {
            input: None,
            input_file: None,
            value: None,
            output: None,
            no_export: false,
            materialize: false,
            state_path: Some(&cell.path),
            cell: None,
            watch: false,
            workers: None,
            verbose,
        },
        config,
    )
    .await
}
