use std::{
    error::Error as _,
    io::{self, IsTerminal},
    path::Path,
    process::ExitCode,
};

use clap::{CommandFactory, Parser};
use kairo_core::Config;
use kairo_runtime::Runtime;
use tracing_subscriber::filter::LevelFilter;

use crate::args::{
    ChaosCommand, Cli, Command, EffectsCommand, NewCommand, StorageCommand, WorkflowCommand,
};

mod args;
mod bench;
mod component;
mod config;
mod discovery;
mod doctor;
mod effect_service;
mod error;
mod execution;
mod inspection;
mod lifecycle;
mod new;
mod prompt;
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

pub(crate) use error::{CliError, Result};

fn root_command() -> clap::Command {
    let command = Cli::command().after_help(
        "Start here:\n  kairo init\n  kairo run <workflow>\n  kairo inspect\n\nCommon commands: init, workflow create, run, runs, inspect, tui\nOperations: up, down, workers, doctor, storage, signal, cancel, prune, chaos",
    );
    if color_enabled(io::stdout().is_terminal()) {
        command.before_help(format!("\x1b[38;5;45m{BANNER}\x1b[0m"))
    } else {
        command
    }
}

fn color_enabled(terminal: bool) -> bool {
    terminal && std::env::var_os("NO_COLOR").is_none()
}

static QUIET: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static JSON_OUTPUT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// shared by `kairo workflows <path>` (kept for backwards compatibility) and the shorter `kairo
/// workflow show <path>` -- both validate and print the same graph.
fn show_workflow(path: &Path, config: Config) -> Result<()> {
    let runtime = Runtime::new(config)?;
    let path = discovery::resolve(path, config)?;
    let workflow = runtime.load_workflow(&path)?;
    runtime.validate_workflow(&workflow)?;
    inspection::print_workflow(&runtime, &workflow, &path);
    Ok(())
}

pub(crate) fn status(color: &str, symbol: &str, message: &str) {
    let terminal = io::stderr().is_terminal();
    if !terminal || QUIET.load(std::sync::atomic::Ordering::Relaxed) {
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

// `kairo bench <workflow>` is sugar for `kairo bench run <workflow>`, rewritten here so
// `BenchCommand`'s clap shape doesn't need a parallel flattened copy of `run`'s fields.
fn normalized_args() -> Vec<std::ffi::OsString> {
    let mut arguments: Vec<_> = std::env::args_os().collect();
    if let Some(bench_index) = arguments.iter().position(|argument| argument == "bench") {
        let next = arguments
            .get(bench_index + 1)
            .and_then(|value| value.to_str());
        if !matches!(next, None | Some("run" | "list" | "show" | "-h" | "--help")) {
            arguments.insert(bench_index + 1, "run".into());
        }
    }
    arguments
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

    let cli = Cli::parse_from(normalized_args());
    QUIET.store(cli.quiet, std::sync::atomic::Ordering::Relaxed);
    JSON_OUTPUT.store(cli.json, std::sync::atomic::Ordering::Relaxed);
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
    let json = cli.json;

    match cli.command {
        None => print_root_help()?,
        Some(Command::Run {
            path,
            file_input,
            input,
            input_file,
            output,
            no_export,
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
                    input_file: file_input.as_deref().or(input_file.as_deref()),
                    output: output.as_deref(),
                    no_export,
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
        Some(Command::Workflows { path }) => match path {
            Some(path) => show_workflow(&path, config)?,
            None => inspection::print_workflows(config)?,
        },
        Some(Command::Cells { workflow }) => inspection::print_cells(workflow.as_deref(), json)?,
        Some(Command::Workers) => service::print_workers()?,
        Some(Command::Chaos {
            command: ChaosCommand::Kill { worker },
        }) => {
            let endpoint = kairo_control::load_endpoint(Path::new(".kairo"))?;
            kairo_control::kill_worker(&endpoint, worker)?;
            print_valid("worker terminated; recovery begins after lease expiry".to_owned());
        }
        Some(Command::Signal { run, signal }) => service::signal(&run, signal.as_deref())?,
        Some(Command::Cancel { run }) => service::cancel(&run)?,
        Some(Command::Prune {
            older_than_hours,
            workflow,
            yes,
        }) => inspection::prune(inspection::PruneOptions {
            older_than_hours,
            workflow,
            yes,
        })?,
        Some(Command::Bench { command }) => bench::dispatch(command, config.allow_console)?,
        Some(Command::Doctor {
            json: local_json,
            fix,
        }) => doctor::run(json || local_json, fix).await?,
        Some(Command::Effects {
            command:
                EffectsCommand::Serve {
                    database,
                    response_delay_ms,
                },
        }) => {
            effect_service::serve(&database, response_delay_ms).map_err(CliError::Effect)?;
        }
        Some(Command::Inspect {
            cell,
            verify,
            export,
        }) => inspection::print_cell(cell.as_deref(), verify, verbose, export.as_deref()).await?,
        Some(Command::Init {
            local,
            minio,
            r2,
            endpoint,
            bucket,
            no_storage,
        }) => {
            let result = setup::initialize(local, minio, r2, endpoint, bucket, no_storage)?;
            let local_storage = result.storage.as_ref().is_some_and(|storage| storage.local);
            let verified = if local_storage {
                Some(setup::check_storage().await?)
            } else {
                setup::check_storage().await.ok()
            };
            setup::report_initialized(result, verified);
        }
        Some(Command::Storage {
            command: StorageCommand::Check { input },
        }) => {
            let check = setup::check_storage().await?;
            print_valid(format!(
                "{} artifact storage · write/read verified · {}",
                check.backend, check.hash
            ));
            if let Some(input) = input {
                setup::check_storage_input(&input).await?;
            }
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
                    durability,
                    wait,
                    effect,
                    advanced,
                },
        }) => {
            let created = new::interactive(
                new::CreateOptions {
                    name,
                    components,
                    input,
                    run,
                    durability,
                    wait,
                    effect,
                    advanced,
                },
                config,
            )?;
            if created.run {
                execution::run_path(
                    &created.path,
                    execution::RunOptions {
                        input: None,
                        input_file: None,
                        output: None,
                        no_export: false,
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
        Some(Command::Workflow {
            command: WorkflowCommand::Show { path },
        }) => show_workflow(&path, config)?,
        Some(Command::Workflow {
            command: WorkflowCommand::Profile { path, repetitions },
        }) => bench::profile_and_report(&path, repetitions, config.allow_console)?,
        Some(Command::Start {
            workers,
            foreground,
        }) => lifecycle::start_with_console(workers.get(), foreground, config.allow_console)?,
        Some(Command::Stop) => lifecycle::stop()?,
        Some(Command::Up { workers }) => {
            let workers = workers
                .map(|workers| workers.get())
                .or(setup::project_workers()?)
                .unwrap_or(2);
            lifecycle::start_with_console(workers, false, config.allow_console)?
        }
        Some(Command::Down) => lifecycle::stop()?,
        Some(Command::RunComponent { path, input }) => {
            execution::run_component(&path, input.unwrap_or_default(), config).await?
        }
        Some(Command::Component { command }) => component::dispatch(command, config)?,
        Some(Command::Worker { id }) => {
            let endpoint = kairo_control::load_endpoint(Path::new(".kairo"))?;
            kairo_worker::run(endpoint, id, config.allow_console)?;
        }
        Some(Command::Serve { workers }) => {
            service::serve(workers.get(), config.allow_console)?;
        }
    }
    Ok(())
}

fn render_error(error: &CliError) {
    if JSON_OUTPUT.load(std::sync::atomic::Ordering::Relaxed) {
        // `doctor` already emits one complete JSON diagnostic object of its own on every
        // outcome, success or failure; a second `{"error": ...}` object would break the
        // single-JSON-document guarantee `--json` promises callers.
        if matches!(error, CliError::Doctor) {
            return;
        }
        let mut messages = vec![error.to_string()];
        let mut source = error.source();
        while let Some(cause) = source {
            messages.push(cause.to_string());
            source = cause.source();
        }
        println!("{}", serde_json::json!({ "error": messages.join(": ") }));
        return;
    }
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
            let code = error.exit_code();
            render_error(&error);
            ExitCode::from(code)
        }
    }
}
