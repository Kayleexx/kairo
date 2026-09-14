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

    /// print machine-readable JSON instead of human-formatted text, where supported.
    #[arg(long, global = true)]
    pub(crate) json: bool,

    /// suppress human status lines; the primary result still prints.
    #[arg(long, global = true)]
    pub(crate) quiet: bool,

    #[command(subcommand)]
    pub(crate) command: Option<Command>,
}

#[derive(Subcommand)]
pub(crate) enum Command {
    /// run a component or workflow.
    #[command(display_order = 3)]
    Run {
        #[arg(default_value = "workflow.yaml")]
        path: PathBuf,
        /// file input for a stream workflow.
        #[arg(value_name = "INPUT", conflicts_with = "input_file")]
        file_input: Option<PathBuf>,
        /// pass an unsigned integer to a component.
        #[arg(long)]
        input: Option<u32>,
        /// read a file as a byte stream; retained for scripts.
        #[arg(long, value_name = "FILE")]
        input_file: Option<PathBuf>,
        /// export a declared workflow output artifact to PATH.
        #[arg(short = 'o', long, value_name = "PATH", conflicts_with = "no_export")]
        output: Option<PathBuf>,
        /// persist a declared output artifact without exporting a local file.
        #[arg(long, conflicts_with = "output")]
        no_export: bool,
        /// use a full in-memory intermediate as a local comparison baseline.
        #[arg(long)]
        materialize: bool,
        /// record this run at FILE; omit FILE to use .kairo/<name>.db.
        #[arg(long, value_name = "FILE", num_args = 0..=1, default_missing_value = "-")]
        state: Option<PathBuf>,
        /// name this run; scalar workflows can resume durable state.
        #[arg(
            long = "run",
            visible_alias = "cell",
            value_name = "NAME",
            conflicts_with = "state"
        )]
        cell: Option<String>,
        /// show compact live activity while this workflow runs.
        #[arg(long)]
        watch: bool,
        /// workers to start for `--watch` when no local service is running.
        #[arg(long, requires = "watch", value_name = "COUNT")]
        workers: Option<NonZeroUsize>,
    },

    /// validate a component or workflow.
    #[command(display_order = 20)]
    Check { path: PathBuf },

    /// show a workflow graph and its edge durability.
    #[command(display_order = 20)]
    Workflows {
        /// workflow file to validate and visualize. omit to list observed workflows.
        path: Option<PathBuf>,
    },

    /// list local workflow runs.
    #[command(name = "runs", visible_alias = "cells", display_order = 4)]
    Cells {
        /// show only runs recorded for this workflow.
        workflow: Option<String>,
    },

    /// list local workers when the service is running.
    #[command(display_order = 20)]
    Workers,

    /// inject a real local worker failure for recovery testing.
    #[command(display_order = 30)]
    Chaos {
        #[command(subcommand)]
        command: ChaosCommand,
    },

    /// deliver a signal to a waiting workflow.
    #[command(display_order = 20)]
    Signal {
        /// run name; a single pending signal can be inferred.
        run: String,
        /// signal name for scripts or when inference is not possible.
        signal: Option<String>,
    },

    /// cancel a run.
    #[command(display_order = 20)]
    Cancel {
        /// run name shown by `kairo runs`, or a workflow name with one cancelable run.
        run: String,
    },

    /// remove completed or failed local run journals.
    #[command(display_order = 20)]
    Prune {
        /// only remove runs untouched for at least this many hours.
        #[arg(long, value_name = "HOURS")]
        older_than_hours: Option<u64>,
        /// only remove runs recorded for this workflow.
        #[arg(long, value_name = "NAME")]
        workflow: Option<String>,
        /// actually delete; without this, only reports what would be removed.
        #[arg(short = 'y', long)]
        yes: bool,
    },

    /// check local setup and explain problems.
    #[command(display_order = 20)]
    Doctor {
        /// print the diagnostic result as JSON.
        #[arg(long)]
        json: bool,
    },

    /// run the local idempotent effect demo service.
    #[command(display_order = 30)]
    Effects {
        #[command(subcommand)]
        command: EffectsCommand,
    },

    /// inspect a local workflow run; defaults to the most recent run.
    #[command(display_order = 5)]
    Inspect {
        /// run name shown by `kairo runs`, or an explicit journal path.
        cell: Option<PathBuf>,
        /// verify recorded checkpoints against the configured artifact store.
        #[arg(long)]
        verify: bool,
        /// re-export this run's output artifact to PATH without rerunning it.
        #[arg(long, value_name = "PATH")]
        export: Option<PathBuf>,
    },

    /// initialize Kairo in this project.
    #[command(display_order = 1)]
    Init {
        /// use a local filesystem artifact store.
        #[arg(long)]
        local: bool,
        /// start a local MinIO artifact store with Docker.
        #[arg(long)]
        minio: bool,
        /// configure Cloudflare R2 using environment values or terminal prompts.
        #[arg(long)]
        r2: bool,
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
    #[command(display_order = 20)]
    Storage {
        #[command(subcommand)]
        command: StorageCommand,
    },

    /// view workflow activity in the terminal.
    #[command(display_order = 6)]
    Tui,

    /// create a runnable workflow from an existing component.
    #[command(display_order = 30)]
    New {
        #[command(subcommand)]
        command: NewCommand,
    },

    /// create a validated workflow with a guided prompt.
    #[command(display_order = 2)]
    Workflow {
        #[command(subcommand)]
        command: WorkflowCommand,
    },

    /// start a local Kairo service and workers.
    #[command(display_order = 20)]
    Start {
        /// number of workers to start.
        #[arg(long, default_value_t = DEFAULT_WORKERS)]
        workers: NonZeroUsize,
        /// keep the service attached to this terminal for troubleshooting.
        #[arg(long)]
        foreground: bool,
    },

    /// stop the local Kairo service and its workers.
    #[command(display_order = 20)]
    Stop,

    /// start a persistent local runtime using project defaults.
    #[command(display_order = 7)]
    Up {
        /// override the local worker count for this service.
        #[arg(long, value_name = "COUNT")]
        workers: Option<NonZeroUsize>,
    },

    /// stop the persistent local runtime.
    #[command(display_order = 8)]
    Down,

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

    #[command(hide = true)]
    Serve {
        #[arg(long)]
        workers: NonZeroUsize,
    },
}

#[derive(Subcommand)]
pub(crate) enum ComponentCommand {
    /// validate and compile a component with wasmtime.
    Check { path: PathBuf },
}

#[derive(Subcommand)]
pub(crate) enum NewCommand {
    /// create NAME.yaml using an existing WebAssembly Component.
    Workflow {
        /// workflow name and output filename.
        name: String,
        /// component file to run in the generated workflow.
        #[arg(long, value_name = "FILE")]
        component: PathBuf,
        /// input value for the generated workflow.
        #[arg(long, default_value_t = 0)]
        input: u32,
    },
}

#[derive(Subcommand)]
pub(crate) enum WorkflowCommand {
    /// build a workflow from one or more Components.
    Create {
        /// workflow name; omit to answer it interactively.
        #[arg(long)]
        name: Option<String>,
        /// Component path; repeat for multiple linear steps.
        #[arg(long = "component")]
        components: Vec<PathBuf>,
        /// scalar input value.
        #[arg(long, default_value_t = 0)]
        input: u32,
        /// run the generated workflow after saving it.
        #[arg(long)]
        run: bool,
        /// durability for generated edges.
        #[arg(long, value_parser = ["ephemeral", "required"])]
        durability: Option<String>,
        /// optional wait: `timer:MS` or `signal:NAME`.
        #[arg(long)]
        wait: Option<String>,
        /// optional idempotent effect operation.
        #[arg(long)]
        effect: Option<String>,
        /// ask about durability, waits, and external effects.
        #[arg(long)]
        advanced: bool,
    },
}

#[derive(Subcommand)]
pub(crate) enum StorageCommand {
    /// verify the configured artifact store can write and read an artifact.
    Check {
        /// also ingest this local file and verify a byte-identical round trip.
        #[arg(long, value_name = "PATH")]
        input: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
pub(crate) enum ChaosCommand {
    /// terminate a registered local worker process.
    Kill { worker: String },
}

#[derive(Subcommand)]
pub(crate) enum EffectsCommand {
    /// serve the local HTTP + SQLite effect provider.
    Serve {
        #[arg(long, default_value = ".kairo/effects.sqlite")]
        database: PathBuf,
        /// delay responses after committing, for crash recovery testing.
        #[arg(long, default_value_t = 0, hide = true)]
        response_delay_ms: u64,
    },
}
