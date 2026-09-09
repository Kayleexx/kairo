use std::{
    error::Error as _,
    io::{self, IsTerminal},
    path::Path,
    process::ExitCode,
};

use clap::{CommandFactory, Parser};
use kairo_core::Config;
use kairo_runtime::Runtime;
use thiserror::Error;
use tracing_subscriber::filter::LevelFilter;

use crate::args::{
    ChaosCommand, Cli, Command, ComponentCommand, EffectsCommand, NewCommand, StorageCommand,
    WorkflowCommand,
};

mod args;
mod config;
mod effect_service;
mod execution;
mod inspection;
mod lifecycle;
mod new;
mod receipts;
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
    #[error("`--watch` can only be used with a scalar workflow")]
    Watch,
    #[error(
        "`--workers` cannot change an already running service; omit it or restart with `kairo start --workers COUNT`"
    )]
    WatchWorkers,
    #[error("local service has no connected workers; run `kairo start --workers COUNT`")]
    NoWorkers,
    #[error("local service thread stopped unexpectedly")]
    ServiceThread,
    #[error(
        "local service was started by an older Kairo; stop it with Ctrl-C, then run `kairo start`"
    )]
    ServiceUpgrade,
    #[error(transparent)]
    Storage(#[from] kairo_storage::StorageError),
    #[error(transparent)]
    Setup(#[from] setup::SetupError),
    #[error(transparent)]
    Inspection(#[from] inspection::InspectionError),
    #[error(transparent)]
    StatePath(#[from] state::StateError),
    #[error(transparent)]
    New(#[from] new::NewError),
    #[error(transparent)]
    Tui(#[from] kairo_tui::TuiError),
    #[error(transparent)]
    Control(#[from] kairo_control::ControlError),
    #[error("failed to start local worker")]
    StartWorker {
        #[source]
        source: std::io::Error,
    },
    #[error("effect service failed: {0}")]
    Effect(String),
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
            watch,
            workers,
        }) => {
            execution::run_path(
                &path,
                execution::RunOptions {
                    input,
                    input_file: input_file.as_deref(),
                    materialize,
                    state_path: state.as_deref(),
                    cell: cell.as_deref(),
                    watch,
                    workers: workers.map(|workers| workers.get()),
                },
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
        Some(Command::Chaos {
            command: ChaosCommand::Kill { worker },
        }) => {
            let endpoint = kairo_control::load_endpoint(Path::new(".kairo"))?;
            kairo_control::kill_worker(&endpoint, worker)?;
            print_valid("worker terminated; recovery begins after lease expiry".to_owned());
        }
        Some(Command::Signal { run, signal }) => service::signal(&run, &signal)?,
        Some(Command::Effects {
            command:
                EffectsCommand::Serve {
                    database,
                    response_delay_ms,
                },
        }) => {
            effect_service::serve(&database, response_delay_ms).map_err(CliError::Effect)?;
        }
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
        Some(Command::Tui) => {
            lifecycle::start(2, false)?;
            kairo_tui::run()?
        }
        Some(Command::New {
            command:
                NewCommand::Workflow {
                    name,
                    component,
                    input,
                },
        }) => new::workflow(&name, &component, input)?,
        Some(Command::Workflow {
            command:
                WorkflowCommand::Create {
                    name,
                    components,
                    input,
                    run,
                },
        }) => {
            let created = new::interactive(name, components, input, run, config)?;
            if created.run {
                execution::run_path(
                    &created.path,
                    execution::RunOptions {
                        input: None,
                        input_file: None,
                        materialize: false,
                        state_path: None,
                        cell: None,
                        watch: io::stdin().is_terminal(),
                        workers: None,
                    },
                    config,
                )
                .await?;
            }
        }
        Some(Command::Start {
            workers,
            foreground,
        }) => lifecycle::start(workers.get(), foreground)?,
        Some(Command::Stop) => lifecycle::stop()?,
        Some(Command::RunComponent { path, input }) => {
            execution::run_component(&path, input.unwrap_or_default(), config).await?
        }
        Some(Command::Component {
            command: ComponentCommand::Check { path },
        }) => validation::component(&path, config)?,
        Some(Command::Worker { id }) => {
            let endpoint = kairo_control::load_endpoint(Path::new(".kairo"))?;
            kairo_worker::run(endpoint, id)?;
        }
        Some(Command::Serve { workers }) => service::serve(workers.get())?,
    }
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
