use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "kairo",
    about = "Run reliable workflows with WebAssembly Components"
)]
#[command(version)]
pub(crate) struct Cli {
    /// show runtime diagnostics.
    #[arg(short, long, global = true)]
    pub(crate) verbose: bool,

    /// allow components to write numeric values to stderr.
    #[arg(long, global = true)]
    pub(crate) allow_console: bool,

    #[command(subcommand)]
    pub(crate) command: Option<Command>,
}

#[derive(Subcommand)]
pub(crate) enum Command {
    /// run a component or workflow.
    Run {
        #[arg(default_value = "workflow.yaml")]
        path: PathBuf,
        /// pass an unsigned integer to a component.
        #[arg(long)]
        input: Option<u32>,
        /// read a file as a byte stream for a stream workflow.
        #[arg(long, value_name = "FILE")]
        input_file: Option<PathBuf>,
        /// use a full in-memory intermediate as a local comparison baseline.
        #[arg(long)]
        materialize: bool,
        /// persist and recover this scalar workflow. omit a path to use
        /// .kairo/<name>.db in the current directory.
        #[arg(long, value_name = "FILE", num_args = 0..=1, default_missing_value = "-")]
        state: Option<PathBuf>,
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
pub(crate) enum ComponentCommand {
    /// validate and compile a component with wasmtime.
    Check { path: PathBuf },
}
