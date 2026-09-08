use std::{num::NonZeroUsize, path::PathBuf};

use clap::{Parser, Subcommand};

const DEFAULT_WORKERS: NonZeroUsize = match NonZeroUsize::new(2) {
    Some(value) => value,
    None => NonZeroUsize::MIN,
};

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
        /// resume or name a durable run.
        #[arg(
            long = "run",
            visible_alias = "cell",
            value_name = "NAME",
            conflicts_with = "state"
        )]
        cell: Option<String>,
    },

    /// validate a component or workflow.
    Check { path: PathBuf },

    /// show a workflow graph and its edge durability.
    Workflows {
        /// workflow file to validate and visualize. omit to list observed workflows.
        path: Option<PathBuf>,
    },

    /// list local workflow runs.
    #[command(name = "runs", visible_alias = "cells")]
    Cells {
        /// show only runs recorded for this workflow.
        workflow: Option<String>,
    },

    /// list local workers when the service is running.
    Workers,

    /// inspect a local workflow run; defaults to the most recent run.
    Inspect {
        /// run name shown by `kairo runs`, or an explicit journal path.
        cell: Option<PathBuf>,
        /// verify recorded checkpoints against the configured artifact store.
        #[arg(long)]
        verify: bool,
    },

    /// configure durable artifact storage.
    Init {
        /// use a local filesystem artifact store.
        #[arg(long)]
        local: bool,
        /// start a local MinIO artifact store with Docker.
        #[arg(long)]
        minio: bool,
        /// configure an external S3-compatible endpoint.
        #[arg(long, value_name = "URL")]
        endpoint: Option<String>,
        /// artifact bucket for an external endpoint.
        #[arg(long, value_name = "NAME")]
        bucket: Option<String>,
        /// skip artifact storage configuration.
        #[arg(long)]
        no_storage: bool,
    },

    /// manage durable artifact storage.
    Storage {
        #[command(subcommand)]
        command: StorageCommand,
    },

    /// view workflow activity in the terminal.
    Tui,

    /// start a local Kairo service and workers.
    Start {
        /// number of workers to start.
        #[arg(long, default_value_t = DEFAULT_WORKERS)]
        workers: NonZeroUsize,
    },

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

    #[command(hide = true)]
    Worker {
        #[arg(long)]
        id: String,
    },
}

#[derive(Subcommand)]
pub(crate) enum ComponentCommand {
    /// validate and compile a component with wasmtime.
    Check { path: PathBuf },
}

#[derive(Subcommand)]
pub(crate) enum StorageCommand {
    /// verify the configured artifact store can write and read an artifact.
    Check,
}
