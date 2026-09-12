use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use kairo_runtime::Runtime;
use kairo_storage::ArtifactStore;

use crate::{CliError, Result, status};

#[allow(clippy::too_many_arguments)]
pub(crate) async fn run(
    runtime: &Runtime,
    workflow: &kairo_core::Workflow,
    input_file: Option<&Path>,
    materialize: bool,
    state_path: Option<&Path>,
    artifacts: Option<&ArtifactStore>,
    output_path: Option<&Path>,
    no_export: bool,
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
        .run_stream_workflow_with_artifacts(workflow, input_file, materialize, artifacts)
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
        run.complete_with_outputs(
            result.duration,
            result.bytes,
            result.checksum,
            result.metrics,
            Some(&result.input_hash),
            &result.values,
            &result.outputs,
        )?;
    }
    let export_path = output_path
        .map(PathBuf::from)
        .or_else(|| {
            workflow
                .output()
                .map(|output| PathBuf::from(&output.filename))
        })
        .filter(|_| !no_export);
    if let Some(path) = &export_path {
        let artifact = result.outputs.first().ok_or(CliError::Output)?;
        let artifacts = artifacts.ok_or(CliError::Output)?;
        if let Err(error) = export(artifacts, &artifact.hash, path).await {
            status(
                "32",
                "✓",
                &format!("{} completed; artifact persisted", workflow.name()),
            );
            return Err(error);
        }
        if let Some(run) = &mut run {
            run.mark_output_exported(0, &path.display().to_string())?;
        }
    }
    if let Some(artifact) = result.outputs.first() {
        status("32", "✓", &format!("{} completed", workflow.name()));
        println!("\n  input   {logical_input}");
        match export_path {
            Some(path) => println!(
                "  output  {}",
                display_output_path(&path, output_path.is_none())
            ),
            None => println!("  output  persisted {}", artifact.filename),
        }
        println!("  size    {}", format_bytes(artifact.bytes));
        println!("  time    {}", format_duration(result.duration));
    } else if !result.values.is_empty() {
        status(
            "32",
            "✓",
            &format!("completed {} in {:?}", workflow.name(), result.duration),
        );
        println!(
            "{}",
            result
                .values
                .iter()
                .map(|value| format!("{} {}", value.value, value.name))
                .collect::<Vec<_>>()
                .join(" · ")
        );
    } else if let Some(labels) = workflow.stream_result_labels() {
        status(
            "32",
            "✓",
            &format!("completed {} in {:?}", workflow.name(), result.duration),
        );
        println!(
            "{} {} · {} {}",
            result.bytes, labels.high, result.checksum, labels.low
        );
    } else {
        status(
            "32",
            "✓",
            &format!("completed {} in {:?}", workflow.name(), result.duration),
        );
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

fn display_output_path(path: &Path, default: bool) -> String {
    if default {
        format!("./{}", path.display())
    } else {
        path.display().to_string()
    }
}

fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} bytes")
    } else {
        format!("{:.1} KiB", bytes as f64 / 1024.0)
    }
}

fn format_duration(duration: std::time::Duration) -> String {
    if duration.as_millis() > 0 {
        format!("{} ms", duration.as_millis())
    } else {
        format!("{} us", duration.as_micros())
    }
}

async fn export(artifacts: &ArtifactStore, hash: &str, destination: &Path) -> Result<()> {
    if destination.exists() {
        return Err(CliError::OutputExists {
            path: destination.to_path_buf(),
        });
    }
    let temporary = temporary_export_path(destination);
    let bytes = artifacts.get_bytes(hash).await?;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|source| CliError::Export {
            path: temporary.clone(),
            source,
        })?;
    let write = file.write_all(&bytes).and_then(|()| file.sync_all());
    if let Err(source) = write {
        let _ = fs::remove_file(&temporary);
        return Err(CliError::Export {
            path: temporary,
            source,
        });
    }
    drop(file);
    fs::hard_link(&temporary, destination).map_err(|source| {
        let _ = fs::remove_file(&temporary);
        CliError::Export {
            path: destination.to_path_buf(),
            source,
        }
    })?;
    fs::remove_file(&temporary).map_err(|source| CliError::Export {
        path: temporary,
        source,
    })?;
    Ok(())
}

fn temporary_export_path(destination: &Path) -> PathBuf {
    let name = destination
        .file_name()
        .unwrap_or(destination.as_os_str())
        .to_string_lossy();
    destination.with_file_name(format!(".{name}.kairo-tmp-{}", std::process::id()))
}
