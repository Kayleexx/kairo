use std::{num::NonZeroUsize, path::PathBuf};

use clap::{Parser, Subcommand};

mod subcommands;

pub(crate) use subcommands::{
    BenchCommand, ChaosCommand, ComponentCommand, EffectsCommand, NewCommand, StorageCommand,
    WorkflowCommand,
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
        /// input for the workflow -- a file path for a stream workflow, or a literal value for a
        /// value workflow; inferred from the workflow's own declared input mode.
        #[arg(value_name = "INPUT", conflicts_with_all = ["input_file", "value"])]
        file_input: Option<PathBuf>,
        /// pass an unsigned integer to a component.
        #[arg(long)]
        input: Option<u32>,
        /// read a file as a byte stream; retained for scripts.
        #[arg(long, value_name = "FILE")]
        input_file: Option<PathBuf>,
        /// pass a literal value to a workflow whose input is `io: input: value` -- driven by the
        /// workflow's own declared input metadata, not just `mode: value`.
        #[arg(long, value_name = "TEXT", conflicts_with_all = ["input", "input_file"])]
        value: Option<String>,
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

    /// show whether the local runtime is ready.
    #[command(display_order = 9)]
    Status,

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

    /// resume an interrupted durable run from its last safe point.
    #[command(display_order = 9)]
    Resume {
        /// run name shown by `kairo runs`, or a workflow name with one resumable run.
        run: String,
    },

    /// create a child run from a completed stream run's durable lineage.
    #[command(display_order = 10)]
    Replay {
        /// completed source run shown by `kairo runs`.
        run: String,
        /// execute through this source workflow step.
        #[arg(long, value_name = "STEP")]
        until: String,
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

    /// measure real repeated executions of a workflow.
    #[command(display_order = 25)]
    Bench {
        #[command(subcommand)]
        command: BenchCommand,
    },

    /// check local setup and explain problems.
    #[command(display_order = 20)]
    Doctor {
        /// print the diagnostic result as JSON.
        #[arg(long)]
        json: bool,
        /// repair fixable issues (missing storage config, a stale local service) instead of
        /// only reporting them.
        #[arg(long)]
        fix: bool,
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

    /// explain the real placement/transport/durability/cost decisions made for a run, built only
    /// from what was actually persisted -- never a recomputed second opinion.
    #[command(display_order = 6)]
    Explain {
        /// run name shown by `kairo runs`, or an explicit journal path.
        cell: Option<PathBuf>,
        /// print machine-readable JSON instead of human-formatted text.
        #[arg(long)]
        json: bool,
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

    /// register and vendor a webassembly component in this project.
    #[command(display_order = 2)]
    Add {
        /// local `.wasm` Component or OCI reference such as `ghcr.io/acme/component:v1`.
        source: String,
        /// friendly catalog name; defaults to the source file name.
        #[arg(long)]
        name: Option<String>,
        /// component version when the source does not provide one.
        #[arg(long)]
        version: Option<String>,
        /// short description shown while composing workflows.
        #[arg(long)]
        description: Option<String>,
    },

    /// list registered project components.
    #[command(display_order = 2)]
    Components,

    /// manage durable artifact storage.
    #[command(display_order = 20)]
    Storage {
        #[command(subcommand)]
        command: StorageCommand,
    },

    /// view workflow activity in the terminal.
    #[command(display_order = 6)]
    Tui,

    /// create a workflow one step at a time, prompting for everything it needs.
    #[command(display_order = 2)]
    New {
        /// workflow name; omit to be prompted.
        #[arg(value_name = "NAME")]
        name: Option<String>,
        /// create from a project recipe in `recipes/`.
        #[arg(long, value_name = "RECIPE")]
        recipe: Option<String>,
        #[command(subcommand)]
        command: Option<NewCommand>,
    },

    /// list reusable project workflow recipes.
    #[command(display_order = 2)]
    Recipes,

    /// create a validated workflow with a guided prompt.
    #[command(display_order = 2)]
    Workflow {
        #[command(subcommand)]
        command: WorkflowCommand,
    },

    /// start a persistent local runtime using project defaults.
    #[command(display_order = 7, alias = "start")]
    Up {
        /// override the local worker count for this service.
        #[arg(long, visible_alias = "scale", value_name = "COUNT")]
        workers: Option<NonZeroUsize>,
        /// keep the service attached to this terminal for troubleshooting.
        #[arg(long)]
        foreground: bool,
    },

    /// stop the persistent local runtime.
    #[command(display_order = 8, alias = "stop")]
    Down,

    /// execute a component and print its output.
    #[command(hide = true)]
    RunComponent {
        path: PathBuf,
        #[arg(long)]
        input: Option<u32>,
    },

    /// author and build WebAssembly Components.
    #[command(display_order = 3)]
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
