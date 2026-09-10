use std::path::Path;

use kairo_runtime::Runtime;

use crate::{Result, status};

pub(crate) async fn run(
    runtime: &Runtime,
    workflow: &kairo_core::Workflow,
    input_file: Option<&Path>,
    materialize: bool,
    state_path: Option<&Path>,
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
    let input = input_file
        .or_else(|| workflow.stream_input())
        .ok_or(kairo_runtime::RuntimeError::InvalidStreamWorkflowInput)?;
    let steps: Vec<_> = workflow
        .steps()
        .iter()
        .map(|step| step.id.to_string())
        .collect();
    let labels = workflow
        .stream_result_labels()
        .map(|labels| (labels.high.as_str(), labels.low.as_str()));
    let mut run = state_path
        .map(|path| kairo_runtime::StreamRun::start(path, workflow.name(), input, &steps, labels))
        .transpose()?;
    let result = match runtime
        .run_stream_workflow(workflow, input_file, materialize)
        .await
    {
        Ok(result) => result,
        Err(error) => {
            if let Some(run) = &mut run {
                run.fail(&error.to_string())?;
            }
            return Err(error.into());
        }
    };
    if let Some(run) = &mut run {
        run.complete(
            result.duration,
            result.bytes,
            result.checksum,
            result.metrics,
        )?;
    }
    status(
        "32",
        "✓",
        &format!("completed {} in {:?}", workflow.name(), result.duration),
    );
    if let Some(labels) = workflow.stream_result_labels() {
        println!(
            "{} {} · {} {}",
            result.bytes, labels.high, result.checksum, labels.low
        );
    } else {
        println!("{} bytes · checksum {:08x}", result.bytes, result.checksum);
    }
    status(
        "36",
        "·",
        &format!(
            "streamed {} bytes · largest batch {} bytes · materialized {} bytes",
            result.metrics.source_bytes,
            result.metrics.largest_batch_bytes,
            result.metrics.materialized_bytes
        ),
    );
    Ok(())
}
