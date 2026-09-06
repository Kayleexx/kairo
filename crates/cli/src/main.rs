use std::{
    error::Error as _,
    io::{self, IsTerminal},
    path::{Path, PathBuf},
    process::ExitCode,
};

use clap::{CommandFactory, Parser, Subcommand};
use kairo_core::Config;
use kairo_runtime::Runtime;
use thiserror::Error;
use tracing_subscriber::filter::LevelFilter;

const BANNER: &str = "\
██╗  ██╗ █████╗ ██╗██████╗  ██████╗
██║ ██╔╝██╔══██╗██║██╔══██╗██╔═══██╗
█████╔╝ ███████║██║██████╔╝██║   ██║
██╔═██╗ ██╔══██║██║██╔══██╗██║   ██║
██║  ██╗██║  ██║██║██║  ██║╚██████╔╝
╚═╝  ╚═╝╚═╝  ╚═╝╚═╝╚═╝  ╚═╝ ╚═════╝";

#[derive(Parser)]
#[command(
    name = "kairo",
    about = "Run reliable workflows with WebAssembly Components"
)]
#[command(version)]
struct Cli {
    /// show runtime diagnostics.
    #[arg(short, long, global = true)]
    verbose: bool,

    /// allow components to write numeric values to stderr.
    #[arg(long, global = true)]
    allow_console: bool,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// run a component or workflow.
    Run {
        #[arg(default_value = "workflow.yaml")]
        path: PathBuf,
        /// pass an unsigned integer to a component.
        #[arg(long)]
        input: Option<u32>,
    },

    /// validate a component or workflow.
    Check { path: PathBuf },

    /// execute a component and print its output.
    #[command(hide = true)]
    RunComponent {
        path: PathBuf,
        #[arg(long)]
        input: Option<u32>,
    },

    /// work with webassembly components.
    #[command(hide = true)]
    Component {
        #[command(subcommand)]
        command: ComponentCommand,
    },
}

#[derive(Subcommand)]
enum ComponentCommand {
    /// validate and compile a component with wasmtime.
    Check { path: PathBuf },
}

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
        Some(Command::Run { path, input }) => run_path(&path, input, config).await?,
        Some(Command::Check { path }) => check_path(&path, config)?,
        Some(Command::RunComponent { path, input }) => {
            run_component(&path, input.unwrap_or_default(), config).await?
        }
        Some(Command::Component {
            command: ComponentCommand::Check { path },
        }) => check_component(&path, config)?,
    }
    Ok(())
}

async fn run_path(path: &Path, input: Option<u32>, config: Config) -> Result<()> {
    if is_workflow(path) {
        if input.is_some() {
            return Err(CliError::WorkflowInput);
        }
        run_workflow(path, config).await
    } else {
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

async fn run_workflow(path: &Path, config: Config) -> Result<()> {
    let runtime = Runtime::new(config)?;
    let workflow = runtime.load_workflow(path)?;
    status(
        "36",
        "→",
        &format!(
            "running {} · {} components",
            workflow.name(),
            workflow.steps().len()
        ),
    );
    let result = runtime.run_workflow(&workflow).await?;
    status(
        "32",
        "✓",
        &format!("completed {} in {:?}", workflow.name(), result.duration),
    );
    println!("{}", result.output);
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
