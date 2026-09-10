use std::path::Path;

use kairo_runtime::Runtime;

use crate::{CliError, Result, status};

pub(crate) async fn run(
    runtime: &Runtime,
    workflow: &kairo_core::Workflow,
    input_file: Option<&Path>,
    materialize: bool,
    state_path: Option<&Path>,
) -> Result<()> {
    let input = input_file
        .or_else(|| workflow.stream_input())
        .ok_or_else(|| CliError::MissingStreamInput {
            workflow: workflow.name().to_owned(),
        })?;
    let logical_input = input
        .file_name()
        .unwrap_or(input.as_os_str())
        .to_string_lossy();
    status(
        "36",
        "→",
        &format!(
            "streaming {} · input {} · {} components",
            workflow.name(),
            logical_input,
            workflow.steps().len()
        ),
    );
    let steps: Vec<_> = workflow
        .steps()
        .iter()
        .map(|step| step.id.to_string())
        .collect();
    let labels = workflow
        .stream_result_labels()
        .map(|labels| (labels.high.as_str(), labels.low.as_str()));
    let mut run = state_path
        .map(|path| {
            kairo_runtime::StreamRun::start_with_provenance(
                path,
                workflow.name(),
                &logical_input,
                if input_file.is_some() {
                    "user"
                } else {
                    "bundled"
                },
                workflow.accepts(),
                &steps,
                labels,
            )
        })
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
        run.complete_with_hash(
            result.duration,
            result.bytes,
            result.checksum,
            result.metrics,
            Some(&result.input_hash),
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
