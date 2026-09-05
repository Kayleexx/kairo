use std::{io::IsTerminal, path::PathBuf, process::ExitCode};

use clap::{CommandFactory, Parser, Subcommand};
use kairo_core::Config;
use kairo_runtime::Runtime;
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
    about = "Locality-aware durable execution for WebAssembly Components"
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

fn root_command() -> clap::Command {
    let command = Cli::command();
    if std::io::stdout().is_terminal() {
        command.before_help(format!("\x1b[38;5;45m{BANNER}\x1b[0m"))
    } else {
        command
    }
}

fn print_root_help() -> kairo_core::Result<()> {
    root_command()
        .print_help()
        .map_err(|error| kairo_core::Error::new("print help", error.to_string()))?;
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

fn run() -> kairo_core::Result<()> {
    if root_help_requested() {
        return print_root_help();
    }

    let cli = Cli::parse();
    tracing_subscriber::fmt()
        .with_target(false)
        .without_time()
        .with_max_level(if cli.verbose {
            LevelFilter::INFO
        } else {
            LevelFilter::WARN
        })
        .init();

    match cli.command {
        None => print_root_help()?,
        Some(Command::Component {
            command: ComponentCommand::Check { path },
        }) => {
            Runtime::new(Config::default())?.load_component(&path)?;
            println!("Component is valid: {}", path.display());
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}
