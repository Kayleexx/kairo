use std::{
    error::Error as _,
    io::{self, IsTerminal},
    path::Path,
    process::ExitCode,
};

use clap::{CommandFactory, Parser};
use kairo_core::{Config, WorkflowMode};
use kairo_runtime::Runtime;
use thiserror::Error;
use tracing_subscriber::filter::LevelFilter;

use crate::args::{Cli, Command, ComponentCommand, StorageCommand};

mod args;
mod config;
mod setup;

const BANNER: &str = "\
██╗  ██╗ █████╗ ██╗██████╗  ██████╗
██║ ██╔╝██╔══██╗██║██╔══██╗██╔═══██╗
█████╔╝ ███████║██║██████╔╝██║   ██║
██╔═██╗ ██╔══██║██║██╔══██╗██║   ██║
██║  ██╗██║  ██║██║██║  ██║╚██████╔╝
╚═╝  ╚═╝╚═╝  ╚═╝╚═╝╚═╝  ╚═╝ ╚═════╝";

#[derive(Debug, Error)]
enum CliError {
    #[error("failed to render help")]
    Help {
        #[source]
        source: std::io::Error,
    },
    #[error(transparent)]
    Runtime(#[from] kairo_runtime::RuntimeError),
    #[error("`--input` can only be used with a component")]
    WorkflowInput,
    #[error("`--input-file` can only be used with a stream workflow")]
    StreamInput,
    #[error("`--materialize` can only be used with a stream workflow")]
    Materialize,
    #[error("`--state` can only be used with a scalar workflow")]
    State,
    #[error("a workflow with `durability: required` needs `--state`")]
    DurableState,
    #[error(transparent)]
    Storage(#[from] kairo_storage::StorageError),
    #[error(transparent)]
    Setup(#[from] setup::SetupError),
}

type Result<T> = std::result::Result<T, CliError>;

fn root_command() -> clap::Command {
    let command = Cli::command();
    if color_enabled(io::stdout().is_terminal()) {
        command.before_help(format!("\x1b[38;5;45m{BANNER}\x1b[0m"))
    } else {
        command
    }
}

fn color_enabled(terminal: bool) -> bool {
    terminal && std::env::var_os("NO_COLOR").is_none()
}

fn status(color: &str, symbol: &str, message: &str) {
    let terminal = io::stderr().is_terminal();
    if !terminal {
        return;
    }
    if color_enabled(terminal) {
        eprintln!("  \x1b[{color}m{symbol}\x1b[0m {message}");
    } else {
        eprintln!("  {symbol} {message}");
    }
}

fn print_valid(kind: &str, path: &Path, detail: Option<String>) {
    let detail = detail.map_or_else(String::new, |detail| format!(" · {detail}"));
    if color_enabled(io::stdout().is_terminal()) {
        println!("\x1b[32mvalid\x1b[0m {kind} · {}{detail}", path.display());
    } else {
        println!("valid {kind} · {}{detail}", path.display());
    }
}

/// replaces any character that is not alphanumeric, `-`, or `_` with `_`.
fn sanitize_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// resolves the raw `--state` value into a concrete path.
///
/// - `None`         → no journal (stateless run)
/// - `Some("-")`    → bare `--state`; derive `.kairo/<name>.db` in cwd
/// - `Some(path)`   → explicit path; use as-is
fn resolve_state(raw: Option<&Path>, workflow_name: &str) -> Option<std::path::PathBuf> {
    match raw {
        None => None,
        Some(p) if p == std::path::Path::new("-") => {
            let name = sanitize_name(workflow_name);
            Some(std::path::PathBuf::from(format!(".kairo/{name}.db")))
        }
        Some(p) => Some(p.to_path_buf()),
    }
}

fn is_workflow(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("yaml") || extension.eq_ignore_ascii_case("yml")
        })
}

fn print_root_help() -> Result<()> {
    root_command()
        .print_help()
        .map_err(|source| CliError::Help { source })?;
    println!();
    Ok(())
}

fn root_help_requested() -> bool {
    let mut arguments = std::env::args_os();
    let _program = arguments.next();
    let Some(argument) = arguments.next() else {
        return false;
    };

    matches!(argument.to_str(), Some("-h" | "--help")) && arguments.next().is_none()
}

async fn run() -> Result<()> {
    if root_help_requested() {
        return print_root_help();
    }

    let cli = Cli::parse();
    setup::load_environment()?;
    tracing_subscriber::fmt()
        .with_target(false)
        .without_time()
        .with_writer(io::stderr)
        .with_ansi(color_enabled(io::stderr().is_terminal()))
        .with_max_level(if cli.verbose {
            LevelFilter::INFO
        } else {
            LevelFilter::WARN
        })
        .init();

    let config = Config {
        allow_console: cli.allow_console,
        ..Config::default()
    };

    match cli.command {
        None => print_root_help()?,
        Some(Command::Run {
            path,
            input,
            input_file,
            materialize,
            state,
        }) => {
            run_path(
                &path,
                input,
                input_file.as_deref(),
                materialize,
                state.as_deref(),
                config,
            )
            .await?
        }
        Some(Command::Check { path }) => check_path(&path, config)?,
        Some(Command::Init {
            local,
            minio,
            endpoint,
            bucket,
            no_storage,
        }) => setup::report_initialized(setup::initialize(
            local, minio, endpoint, bucket, no_storage,
        )?),
        Some(Command::Storage {
            command: StorageCommand::Check,
        }) => {
            setup::artifact_store()?.check().await?;
            print_valid("artifact storage", Path::new("configured store"), None);
        }
        Some(Command::RunComponent { path, input }) => {
            run_component(&path, input.unwrap_or_default(), config).await?
        }
        Some(Command::Component {
            command: ComponentCommand::Check { path },
        }) => check_component(&path, config)?,
    }
    Ok(())
}

async fn run_path(
    path: &Path,
    input: Option<u32>,
    input_file: Option<&Path>,
    materialize: bool,
    state: Option<&Path>,
    config: Config,
) -> Result<()> {
    if is_workflow(path) {
        run_workflow(path, input, input_file, materialize, state, config).await
    } else {
        if input_file.is_some() {
            return Err(CliError::StreamInput);
        }
        if materialize {
            return Err(CliError::Materialize);
        }
        if state.is_some() {
            return Err(CliError::State);
        }
        run_component(path, input.unwrap_or_default(), config).await
    }
}

async fn run_component(path: &Path, input: u32, config: Config) -> Result<()> {
    status("36", "→", &format!("running {}", path.display()));
    let runtime = Runtime::new(config)?;
    let component = runtime.load_component(path)?;
    let result = runtime.run_component(&component, input).await?;
    status("32", "✓", &format!("completed in {:?}", result.duration));
    println!("{}", result.output);
    Ok(())
}

async fn run_workflow(
    path: &Path,
    input: Option<u32>,
    input_file: Option<&Path>,
    materialize: bool,
    state: Option<&Path>,
    config: Config,
) -> Result<()> {
    let runtime = Runtime::new(config)?;
    let workflow = runtime.load_workflow(path)?;
    match workflow.mode() {
        WorkflowMode::Scalar => {
            if input.is_some() {
                return Err(CliError::WorkflowInput);
            }
            if input_file.is_some() {
                return Err(CliError::StreamInput);
            }
            if materialize {
                return Err(CliError::Materialize);
            }
            let state = resolve_state(state, workflow.name());
            if workflow.requires_durable_artifacts() && state.is_none() {
                return Err(CliError::DurableState);
            }
            run_scalar_workflow(&runtime, &workflow, state.as_deref()).await
        }
        WorkflowMode::Stream => {
            if input.is_some() {
                return Err(CliError::WorkflowInput);
            }
            if state.is_some() {
                return Err(CliError::State);
            }
            run_stream_workflow(&runtime, &workflow, input_file, materialize).await
        }
    }
}

async fn run_scalar_workflow(
    runtime: &Runtime,
    workflow: &kairo_core::Workflow,
    state: Option<&Path>,
) -> Result<()> {
    status(
        "36",
        "→",
        &format!(
            "running {} · {} components",
            workflow.name(),
            workflow.steps().len()
        ),
    );
    let artifacts = workflow
        .requires_durable_artifacts()
        .then(setup::artifact_store)
        .transpose()?;
    let result = match state {
        Some(path) => runtime.run_cell(workflow, path, artifacts.as_ref()).await?,
        None => runtime.run_workflow(workflow).await?,
    };
    status(
        "32",
        "✓",
        &format!(
            "{} {} in {:?}",
            if result.resumed {
                "resumed and completed"
            } else {
                "completed"
            },
            workflow.name(),
            result.duration
        ),
    );
    println!("{}", result.output);
    Ok(())
}

async fn run_stream_workflow(
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

fn check_path(path: &Path, config: Config) -> Result<()> {
    if is_workflow(path) {
        let runtime = Runtime::new(config)?;
        let workflow = runtime.load_workflow(path)?;
        runtime.validate_workflow(&workflow)?;
        print_valid(
            "workflow",
            path,
            Some(format!("{} components", workflow.steps().len())),
        );
        Ok(())
    } else {
        check_component(path, config)
    }
}

fn check_component(path: &Path, config: Config) -> Result<()> {
    let component = Runtime::new(config)?.load_component(path)?;
    print_valid("component", path, Some(component.hash().to_string()));
    Ok(())
}

fn render_error(error: &CliError) {
    if color_enabled(io::stderr().is_terminal()) {
        eprintln!("\x1b[31merror:\x1b[0m {error}");
    } else {
        eprintln!("error: {error}");
    }
    let mut source = error.source();
    while let Some(cause) = source {
        eprintln!("caused by: {cause}");
        source = cause.source();
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            render_error(&error);
            ExitCode::FAILURE
        }
    }
}
