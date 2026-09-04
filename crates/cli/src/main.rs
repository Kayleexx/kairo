use std::{path::PathBuf, process::ExitCode};

use clap::{CommandFactory, Parser, Subcommand};
use kairo_core::Config;
use kairo_runtime::Runtime;
use tracing_subscriber::filter::LevelFilter;

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

fn run() -> kairo_core::Result<()> {
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
        None => {
            Cli::command()
                .print_help()
                .map_err(|error| kairo_core::Error::new("print help", error.to_string()))?;
            println!();
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

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}
