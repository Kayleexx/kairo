use std::{
    fs,
    io::{self, IsTerminal, Read, Write},
    path::Path,
};

use kairo_core::{Config, IoInput, Workflow};
use kairo_runtime::Runtime;

use super::RunOptions;
use crate::{CliError, Result, setup, state, status};

pub(super) async fn run(
    runtime: &Runtime,
    workflow: &Workflow,
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
    if options.watch {
        return Err(CliError::Watch);
    }
    if options.workers.is_some() {
        return Err(CliError::StreamWorkers);
    }

    let input = resolve_input(workflow, options.input_file, &config)?;

    let mut state_path = state::resolve_run(options.state_path, options.cell, workflow.name())?;
    let durable = workflow.requires_durable_artifacts() || workflow.has_unresolved_durability();
    if state_path.is_none() && durable {
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

    status("36", "→", workflow.name());
    let result = match state_path {
        Some(path) => {
            runtime
                .run_value_cell(workflow, path, artifacts.as_ref(), input)
                .await?
        }
        None => runtime.run_value_workflow(workflow, input).await?,
    };
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
    input_file: Option<&Path>,
    config: &Config,
) -> Result<Vec<u8>> {
    if let Some(path) = input_file {
        return read_input(path, config.max_stream_output_bytes);
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
    if line == "-" || Path::new(line).is_file() {
        return read_input(Path::new(line), config.max_stream_output_bytes);
    }
    Ok(line.as_bytes().to_vec())
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
