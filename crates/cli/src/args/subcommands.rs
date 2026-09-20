use std::path::PathBuf;

use clap::Subcommand;

#[derive(Subcommand)]
pub(crate) enum ComponentCommand {
    /// scaffold a new component project at components/NAME.
    New { name: String },
    /// build a component project into a WebAssembly Component.
    Build {
        /// component project directory.
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// validate and compile a component with wasmtime.
    Check { path: PathBuf },
    /// show a registered component's contract and source.
    Show { name: String },
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
    #[command(alias = "new")]
    Create {
        /// workflow name; omit to answer it interactively.
        #[arg(long)]
        name: Option<String>,
        /// Component path; repeat for multiple linear steps.
        #[arg(long = "component")]
        components: Vec<PathBuf>,
        /// workflow name (if `--name` is omitted) followed by component names, resolved from
        /// `components/<name>/component.wasm`; enables non-interactive creation without flags,
        /// e.g. `kairo workflow new locality warm-up slow-compute finish`.
        #[arg(value_name = "NAME")]
        steps: Vec<String>,
        /// scalar input value.
        #[arg(long, default_value_t = 0)]
        input: u32,
        /// run the generated workflow after saving it.
        #[arg(long)]
        run: bool,
        /// durability for generated edges.
        #[arg(long, value_parser = ["ephemeral", "required", "auto"])]
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
    /// show a workflow's graph and its edge durability.
    Show { path: PathBuf },
    /// measure this workflow's `durability: auto` edges and write a durability profile.
    Profile {
        path: PathBuf,
        /// timed attempts per edge to measure and average.
        #[arg(long, default_value_t = 10, value_name = "COUNT")]
        repetitions: u32,
        /// literal value to profile with, for a workflow whose input is `io: input: value`.
        #[arg(long, value_name = "TEXT")]
        value: Option<String>,
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
pub(crate) enum BenchCommand {
    /// run repeated attempts of a workflow and write a report.
    Run {
        /// workflow file or name to benchmark.
        workflow: PathBuf,
        /// file input for a stream workflow.
        input: Option<PathBuf>,
        /// untimed attempts run first, to warm caches before measuring.
        #[arg(long, default_value_t = 1, value_name = "COUNT")]
        warmups: u32,
        /// timed attempts to measure and summarize; for `--failure-scenario`, the number of
        /// kill-and-recover cycles.
        #[arg(long, default_value_t = 10, value_name = "COUNT")]
        repetitions: u32,
        /// write the report to PATH instead of `.kairo/benchmarks/<workflow>-<time>.json`.
        #[arg(long, value_name = "PATH")]
        output: Option<PathBuf>,
        /// measure real worker-crash recovery instead of plain timing. scalar workflows only;
        /// takes over the local service for the duration of the benchmark.
        #[arg(long, value_parser = ["worker-kill"], value_name = "NAME", conflicts_with = "profile")]
        failure_scenario: Option<String>,
        /// measure this workflow's `durability: auto` edges and write `.kairo/profiles/<shape>.json`.
        #[arg(long, conflicts_with = "failure_scenario")]
        profile: bool,
    },
    /// list saved benchmark reports.
    List,
    /// print a saved benchmark report; defaults to the most recently written one.
    Show {
        /// report filename (under `.kairo/benchmarks/`) or a full path.
        report: Option<PathBuf>,
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
