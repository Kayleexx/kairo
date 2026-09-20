use std::io::{self, IsTerminal};

use clap::CommandFactory;

use crate::{CliError, Result, args::Cli, color_enabled};

const BANNER: &str = "\
██╗  ██╗ █████╗ ██╗██████╗  ██████╗
██║ ██╔╝██╔══██╗██║██╔══██╗██╔═══██╗
█████╔╝ ███████║██║██████╔╝██║   ██║
██╔═██╗ ██╔══██║██║██╔══██╗██║   ██║
██║  ██╗██║  ██║██║██║  ██║╚██████╔╝
╚═╝  ╚═╝╚═╝  ╚═╝╚═╝╚═╝  ╚═╝ ╚═════╝";

fn root_command() -> clap::Command {
    let command = Cli::command().after_help(
        "Start here:\n  kairo init\n  kairo add <component.wasm>\n  kairo new <workflow>\n  kairo check <workflow>\n  kairo run <workflow>\n\nCommon commands: init, add, components, recipes, new, check, run, inspect, explain, tui\nDeveloper tools: component new, component build, workflow create\nOperations: up, down, workers, doctor, storage, signal, cancel, prune, chaos",
    );
    if color_enabled(io::stdout().is_terminal()) {
        command.before_help(format!("\x1b[38;5;45m{BANNER}\x1b[0m"))
    } else {
        command
    }
}

pub(crate) fn print_root_help() -> Result<()> {
    root_command()
        .print_help()
        .map_err(|source| CliError::Help { source })?;
    println!();
    Ok(())
}

// `kairo bench <workflow>` is sugar for `kairo bench run <workflow>`, rewritten here so
// `BenchCommand`'s clap shape doesn't need a parallel flattened copy of `run`'s fields.
pub(crate) fn normalized_args() -> Vec<std::ffi::OsString> {
    let mut arguments: Vec<_> = std::env::args_os().collect();
    if let Some(bench_index) = arguments.iter().position(|argument| argument == "bench") {
        let next = arguments
            .get(bench_index + 1)
            .and_then(|value| value.to_str());
        if !matches!(next, None | Some("run" | "list" | "show" | "-h" | "--help")) {
            arguments.insert(bench_index + 1, "run".into());
        }
    }
    arguments
}

pub(crate) fn root_help_requested() -> bool {
    let mut arguments = std::env::args_os();
    let _program = arguments.next();
    let Some(argument) = arguments.next() else {
        return false;
    };

    matches!(argument.to_str(), Some("-h" | "--help")) && arguments.next().is_none()
}
