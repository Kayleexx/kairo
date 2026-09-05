use std::{error::Error as _, io::IsTerminal, path::PathBuf, process::ExitCode};

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

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// execute a component and print its output.
    RunComponent { path: PathBuf },

    /// work with webassembly components.
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
}

type Result<T> = std::result::Result<T, CliError>;

fn root_command() -> clap::Command {
    let command = Cli::command();
    if std::io::stdout().is_terminal() {
        command.before_help(format!("\x1b[38;5;45m{BANNER}\x1b[0m"))
    } else {
        command
    }
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
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal())
        .with_max_level(if cli.verbose {
            LevelFilter::INFO
        } else {
            LevelFilter::WARN
        })
        .init();

    match cli.command {
        None => print_root_help()?,
        Some(Command::RunComponent { path }) => {
            let runtime = Runtime::new(Config::default())?;
            let component = runtime.load_component(&path)?;
            let result = runtime.run_component(&component).await?;
            println!("{}", result.output);
        }
        Some(Command::Component {
            command: ComponentCommand::Check { path },
        }) => {
            Runtime::new(Config::default())?.load_component(&path)?;
            println!("Component is valid: {}", path.display());
        }
    }
    Ok(())
}

fn render_error(error: &CliError) {
    eprintln!("error: {error}");
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
