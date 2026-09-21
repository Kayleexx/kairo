use std::{
    fs,
    io::{self, IsTerminal, Read, Write},
    path::Path,
    time::Duration,
};

use kairo_core::{Config, IoInput, Workflow};
use kairo_runtime::{Runtime, ValueWorkflowResult};
use kairo_storage::ArtifactStore;

use super::RunOptions;
use crate::{CliError, Result, setup, state, status};

pub(super) async fn run(
    runtime: &Runtime,
    workflow: &Workflow,
    workflow_path: &Path,
    options: RunOptions<'_>,
    config: Config,
) -> Result<()> {
    if options.input.is_some() {
        return Err(CliError::WorkflowInput);
    }
    if options.materialize {
        return Err(CliError::Materialize);
    }
    if options.no_export {
        return Err(CliError::Output);
    }
    if options.workers.is_some() {
        return Err(CliError::ValueWorkers);
    }

    let input = resolve_input(
        workflow,
        options.value,
        options.input_file,
        options.positional,
        &config,
    )?;

    let mut state_path = state::resolve_run(options.state_path, options.cell, workflow.name())?;
    let durable = workflow.requires_durable_artifacts() || workflow.has_unresolved_durability();
    // `--watch` needs a real journal to poll for progress, exactly like a durable run does --
    // it doesn't need a worker pool or the control plane, just somewhere to observe events.
    if state_path.is_none() && (durable || options.watch) {
        state_path = Some(state::generated_run(workflow.name())?);
    }
    let artifacts = if durable {
        if setup::ensure_storage()? {
            status("32", "✓", "local artifact storage ready");
        }
        Some(setup::artifact_store()?)
    } else {
        None
    };
    if workflow.has_unresolved_durability() {
        super::ensure_profiled(
            runtime,
            workflow,
            workflow_path,
            options.value.or_else(|| options.positional?.to_str()),
            config.allow_console,
        )?;
    }

    status("36", "→", workflow.name());
    let observed_state_path = state_path.clone();
    let result = match state_path {
        Some(path) if options.watch => {
            run_and_watch(runtime, workflow, &path, artifacts.as_ref(), input).await?
        }
        Some(path) => {
            runtime
                .run_value_cell(workflow, path, artifacts.as_ref(), input)
                .await?
        }
        None => runtime.run_value_workflow(workflow, input).await?,
    };
    if let Some(path) = &observed_state_path
        && let Err(error) = runtime.record_value_profile_observations(workflow, path)
    {
        tracing::warn!(%error, "failed to record durability profile observations");
    }
    status(
        "32",
        "✓",
        &format!(
            "{} {} in {:?}",
            if result.resumed {
                "restored from journal"
            } else {
                "completed"
            },
            workflow.name(),
            result.duration
        ),
    );

    if let Some(path) = options.output {
        fs::write(path, &result.output).map_err(|source| CliError::Export {
            path: path.to_path_buf(),
            source,
        })?;
        status("32", "✓", &format!("exported to {}", path.display()));
    } else {
        print_output(&result.output);
    }
    Ok(())
}

/// runs a value-mode workflow exactly as `run_value_cell` already does, while concurrently
/// polling its own journal for progress -- the same observable run/event model `kairo inspect`
/// already reads (`inspect_value_cell`), just watched live instead of after the fact. A value
/// workflow never goes through a worker or the control plane (it always runs locally), so
/// "watch" here means "poll the journal while it runs," not "poll a worker's reported status" --
/// the same user-facing `--watch` behavior, reached the way this mode actually executes.
async fn run_and_watch(
    runtime: &Runtime,
    workflow: &Workflow,
    state_path: &Path,
    artifacts: Option<&ArtifactStore>,
    input: Vec<u8>,
) -> Result<ValueWorkflowResult> {
    let execution = runtime.run_value_cell(workflow, state_path, artifacts, input);
    tokio::pin!(execution);
    let mut completed = 0_usize;
    loop {
        tokio::select! {
            result = &mut execution => return Ok(result?),
            () = tokio::time::sleep(Duration::from_millis(50)) => {
                print_progress(state_path, &mut completed);
            }
        }
    }
}

fn print_progress(state_path: &Path, completed: &mut usize) {
    let Ok(Some(inspection)) = kairo_runtime::inspect_value_cell(state_path) else {
        return;
    };
    for component in inspection.components.iter().skip(*completed) {
        if component.output_preview.is_none() {
            break;
        }
        status("32", "✓", &component.name);
        *completed += 1;
    }
}

fn print_output(bytes: &[u8]) {
    match std::str::from_utf8(bytes) {
        Ok(text) => println!("{text}"),
        Err(_) => println!(
            "<{} bytes (binary); use --output <path> to export>",
            bytes.len()
        ),
    }
}

fn resolve_input(
    workflow: &Workflow,
    value: Option<&str>,
    input_file: Option<&Path>,
    positional: Option<&Path>,
    config: &Config,
) -> Result<Vec<u8>> {
    if let Some(text) = value {
        return Ok(text.as_bytes().to_vec());
    }
    if let Some(path) = input_file {
        return read_input(path, config.max_stream_output_bytes);
    }
    if let Some(path) = positional {
        if workflow.io().input == IoInput::None {
            return Err(CliError::UnexpectedInput {
                workflow: workflow.name().to_owned(),
            });
        }
        return path_or_literal(&path.to_string_lossy(), config);
    }
    match workflow.io().input {
        IoInput::None => Ok(Vec::new()),
        _ if io::stdin().is_terminal() => prompt_input(workflow, config),
        _ => Err(CliError::MissingValueInput {
            workflow: workflow.name().to_owned(),
        }),
    }
}

// this workflow's own `io`/`accepts` metadata drives the prompt -- a non-technical user never
// needs to know a flag exists, let alone which one.
fn prompt_input(workflow: &Workflow, config: &Config) -> Result<Vec<u8>> {
    if let Some(description) = workflow.description() {
        println!("{description}");
    }
    let accepts = workflow.accepts();
    let hint = if accepts.is_empty() {
        String::new()
    } else {
        format!(" ({})", accepts.join(", "))
    };
    print!("this workflow expects a value{hint} — provide a file path or type a value: ");
    io::stdout()
        .flush()
        .map_err(|source| CliError::Prompt { source })?;
    let mut line = String::new();
    io::stdin()
        .read_line(&mut line)
        .map_err(|source| CliError::Prompt { source })?;
    let line = line.trim();
    if line.is_empty() {
        return Err(CliError::MissingValueInput {
            workflow: workflow.name().to_owned(),
        });
    }
    path_or_literal(line, config)
}

fn path_or_literal(text: &str, config: &Config) -> Result<Vec<u8>> {
    if text == "-" || Path::new(text).is_file() {
        return read_input(Path::new(text), config.max_stream_output_bytes);
    }
    Ok(text.as_bytes().to_vec())
}

fn read_input(path: &Path, max_bytes: u64) -> Result<Vec<u8>> {
    if path == Path::new("-") {
        let mut bytes = Vec::new();
        io::stdin()
            .take(max_bytes)
            .read_to_end(&mut bytes)
            .map_err(|source| CliError::ReadInput {
                path: path.to_path_buf(),
                source,
            })?;
        return Ok(bytes);
    }
    let metadata = fs::metadata(path).map_err(|source| CliError::ReadInput {
        path: path.to_path_buf(),
        source,
    })?;
    if metadata.len() > max_bytes {
        return Err(CliError::InputTooLarge {
            path: path.to_path_buf(),
            max_bytes,
        });
    }
    fs::read(path).map_err(|source| CliError::ReadInput {
        path: path.to_path_buf(),
        source,
    })
}
