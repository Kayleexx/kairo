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
mod inspection;
mod service;
mod setup;
mod state;
mod stream;
mod validation;

const BANNER: &str = "\
██╗  ██╗ █████╗ ██╗██████╗  ██████╗
██║ ██╔╝██╔══██╗██║██╔══██╗██╔═══██╗
█████╔╝ ███████║██║██████╔╝██║   ██║
██╔═██╗ ██╔══██║██║██╔══██╗██║   ██║
██║  ██╗██║  ██║██║██║  ██║╚██████╔╝
╚═╝  ╚═╝╚═╝  ╚═╝╚═╝╚═╝  ╚═╝ ╚═════╝";

#[derive(Debug, Error)]
pub(crate) enum CliError {
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
    #[error("`--cell` and `--state` can only be used with a scalar workflow")]
    State,
    #[error(transparent)]
    Storage(#[from] kairo_storage::StorageError),
    #[error(transparent)]
    Setup(#[from] setup::SetupError),
    #[error(transparent)]
    Inspection(#[from] inspection::InspectionError),
    #[error(transparent)]
    StatePath(#[from] state::StateError),
    #[error(transparent)]
    Tui(#[from] kairo_tui::TuiError),
    #[error(transparent)]
    Control(#[from] kairo_control::ControlError),
    #[error("failed to start local worker")]
    StartWorker {
        #[source]
        source: std::io::Error,
    },
}

pub(crate) type Result<T> = std::result::Result<T, CliError>;

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

pub(crate) fn status(color: &str, symbol: &str, message: &str) {
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

pub(crate) fn print_valid(message: String) {
    if color_enabled(io::stdout().is_terminal()) {
        println!("\x1b[32mvalid\x1b[0m {message}");
    } else {
        println!("valid {message}");
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
    let verbose = cli.verbose;

    match cli.command {
        None => print_root_help()?,
        Some(Command::Run {
            path,
            input,
            input_file,
            materialize,
            state,
            cell,
        }) => {
            run_path(
                &path,
                input,
                input_file.as_deref(),
                materialize,
                state.as_deref(),
                cell.as_deref(),
                config,
            )
            .await?
        }
        Some(Command::Check { path }) => validation::check(&path, config)?,
        Some(Command::Workflows { path }) => {
            if let Some(path) = path {
                let runtime = Runtime::new(config)?;
                let workflow = runtime.load_workflow(&path)?;
                runtime.validate_workflow(&workflow)?;
                inspection::print_workflow(&workflow, &path);
            } else {
                inspection::print_workflows()?;
            }
        }
        Some(Command::Cells { workflow }) => inspection::print_cells(workflow.as_deref())?,
        Some(Command::Workers) => service::print_workers()?,
        Some(Command::Inspect { cell, verify }) => {
            inspection::print_cell(cell.as_deref(), verify, verbose).await?
        }
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
            let check = setup::check_storage().await?;
            print_valid(format!(
                "{} artifact storage · write/read verified · {}",
                check.backend, check.hash
            ));
        }
        Some(Command::Tui) => kairo_tui::run()?,
        Some(Command::Start { workers }) => service::start(workers.get())?,
        Some(Command::RunComponent { path, input }) => {
            run_component(&path, input.unwrap_or_default(), config).await?
        }
        Some(Command::Component {
            command: ComponentCommand::Check { path },
        }) => validation::component(&path, config)?,
        Some(Command::Worker { id }) => {
            let endpoint = kairo_control::load_endpoint(Path::new(".kairo"))?;
            kairo_worker::run(endpoint, id)?;
        }
    }
    Ok(())
}

async fn run_path(
    path: &Path,
    input: Option<u32>,
    input_file: Option<&Path>,
    materialize: bool,
    state: Option<&Path>,
    cell: Option<&str>,
    config: Config,
) -> Result<()> {
    if is_workflow(path) {
        run_workflow(path, input, input_file, materialize, state, cell, config).await
    } else {
        if input_file.is_some() {
            return Err(CliError::StreamInput);
        }
        if materialize {
            return Err(CliError::Materialize);
        }
        if state.is_some() || cell.is_some() {
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
    cell: Option<&str>,
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
            let mut state = state::resolve_run(state, cell, workflow.name())?;
            if state.is_none() && kairo_control::load_endpoint(Path::new(".kairo")).is_ok() {
                state = Some(state::generated_run(workflow.name()));
            }
            if state.is_none() && workflow.requires_durable_artifacts() {
                state = Some(state::generated_run(workflow.name()));
            }
            run_scalar_workflow(&runtime, &workflow, path, state.as_deref()).await
        }
        WorkflowMode::Stream => {
            if input.is_some() {
                return Err(CliError::WorkflowInput);
            }
            if state.is_some() || cell.is_some() {
                return Err(CliError::State);
            }
            stream::run(&runtime, &workflow, input_file, materialize).await
        }
    }
}

async fn run_scalar_workflow(
    runtime: &Runtime,
    workflow: &kairo_core::Workflow,
    workflow_path: &Path,
    state: Option<&Path>,
) -> Result<()> {
    if let (Ok(endpoint), Some(state)) = (kairo_control::load_endpoint(Path::new(".kairo")), state)
    {
        match service::submit_run(&endpoint, workflow, workflow_path, state).await {
            Ok(()) => return Ok(()),
            Err(CliError::Control(kairo_control::ControlError::Unavailable)) => {}
            Err(error) => return Err(error),
        }
    }
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
                "restored from journal"
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
