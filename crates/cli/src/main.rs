use std::{
    error::Error as _,
    io::{self, IsTerminal},
    path::Path,
    process::ExitCode,
};

use clap::Parser;
use kairo_core::Config;
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
mod explain;
mod help;
mod inspection;
mod lifecycle;
mod new;
mod prompt;
mod receipts;
mod recipe_templates;
mod replay;
mod service;
mod setup;
mod state;
mod stream;
mod validation;

pub(crate) use error::{CliError, Result};

pub(crate) fn color_enabled(terminal: bool) -> bool {
    terminal && std::env::var_os("NO_COLOR").is_none()
}

static QUIET: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static JSON_OUTPUT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

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

async fn run() -> Result<()> {
    if help::root_help_requested() {
        return help::print_root_help();
    }

    let cli = Cli::parse_from(help::normalized_args());
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
        None => help::print_root_help()?,
        Some(Command::Run {
            path,
            file_input,
            input,
            input_file,
            value,
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
                    value: value.as_deref(),
                    output: output.as_deref(),
                    no_export,
                    materialize,
                    state_path: state.as_deref(),
                    cell: cell.as_deref(),
                    watch,
                    workers: workers.map(|workers| workers.get()),
                    verbose,
                },
                config,
            )
            .await?
        }
        Some(Command::Check { path }) => validation::check(&path, config)?,
        Some(Command::Workflows { path }) => match path {
            Some(path) => validation::show_workflow(&path, config)?,
            None => inspection::print_workflows(config)?,
        },
        Some(Command::Cells { workflow }) => inspection::print_cells(workflow.as_deref(), json)?,
        Some(Command::Status) => service::print_status(verbose)?,
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
        Some(Command::Resume { run }) => inspection::resume(&run, config, verbose).await?,
        Some(Command::Replay { run, until }) => replay::run(&run, &until, config, verbose).await?,
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
        Some(Command::Explain { cell, json }) => explain::print(cell.as_deref(), json).await?,
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
        Some(Command::Add {
            source,
            name,
            version,
            description,
        }) => {
            component::add(
                &source,
                name.as_deref(),
                version.as_deref(),
                description.as_deref(),
                config,
            )
            .await?
        }
        Some(Command::Components) => component::list(config),
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
            name,
            recipe,
            command: None,
        }) => {
            if let Some(recipe) = recipe {
                new::from_recipe(name, &recipe, config)?;
            } else {
                new::guided(name, config)?;
            }
        }
        Some(Command::New {
            name: _,
            recipe: _,
            command:
                Some(NewCommand::Workflow {
                    name,
                    component,
                    input,
                }),
        }) => new::workflow(&name, &component, input)?,
        Some(Command::Recipes) => new::list_recipes(config),
        Some(Command::Workflow {
            command:
                WorkflowCommand::Create {
                    name,
                    components,
                    steps,
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
                    steps,
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
                        value: None,
                        output: None,
                        no_export: false,
                        materialize: false,
                        state_path: None,
                        cell: None,
                        watch: io::stdin().is_terminal(),
                        workers: None,
                        verbose,
                    },
                    config,
                )
                .await?;
            }
        }
        Some(Command::Workflow {
            command: WorkflowCommand::Show { path },
        }) => validation::show_workflow(&path, config)?,
        Some(Command::Workflow {
            command:
                WorkflowCommand::Profile {
                    path,
                    repetitions,
                    value,
                },
        }) => {
            bench::profile_and_report(&path, repetitions, value.as_deref(), config.allow_console)?
        }
        Some(Command::Start {
            workers,
            foreground,
        }) => {
            lifecycle::start_with_console(workers.get(), foreground, config.allow_console, verbose)?
        }
        Some(Command::Stop) => lifecycle::stop()?,
        Some(Command::Up { workers }) => {
            let workers = workers
                .map(|workers| workers.get())
                .or(setup::project_workers()?)
                .unwrap_or(2);
            lifecycle::start_with_console(workers, false, config.allow_console, verbose)?
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
            service::serve(workers.get(), config.allow_console, verbose)?;
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
