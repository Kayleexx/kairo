use std::path::Path;

use kairo_runtime::Runtime;

use crate::{Result, status};

pub(crate) async fn run(
    runtime: &Runtime,
    workflow: &kairo_core::Workflow,
    input_file: Option<&Path>,
    materialize: bool,
) -> Result<()> {
    status(
        "36",
        "→",
        &format!(
            "streaming {} · {} components",
            workflow.name(),
            workflow.steps().len()
        ),
    );
    let result = runtime
        .run_stream_workflow(workflow, input_file, materialize)
        .await?;
    status(
        "32",
        "✓",
        &format!("completed {} in {:?}", workflow.name(), result.duration),
    );
    println!("{} bytes · checksum {:08x}", result.bytes, result.checksum);
    Ok(())
}
