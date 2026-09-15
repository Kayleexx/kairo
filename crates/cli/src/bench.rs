use std::path::PathBuf;

mod chaos;
mod config;
mod profile;
mod report;
mod reports;
mod runner;

pub(crate) use config::FailureScenario;
pub(crate) use runner::BenchError;

pub(crate) fn dispatch(command: crate::args::BenchCommand) -> Result<(), BenchError> {
    match command {
        crate::args::BenchCommand::Run {
            workflow,
            repetitions,
            profile: true,
            ..
        } => {
            let workflow = resolve_workflow(&workflow)?;
            let path = self::profile::run(&workflow, repetitions)?;
            crate::status(
                "32",
                "✓",
                &format!("durability profile · {}", path.display()),
            );
            Ok(())
        }
        crate::args::BenchCommand::Run {
            workflow,
            input,
            warmups,
            repetitions,
            output,
            failure_scenario,
            profile: false,
        } => run(
            &resolve_workflow(&workflow)?,
            input.as_deref(),
            warmups,
            repetitions,
            output.as_deref(),
            // clap's `value_parser` accepts only "worker-kill" today, so this is the only case.
            failure_scenario.map(|_| FailureScenario::WorkerKill),
        ),
        crate::args::BenchCommand::List => reports::list(),
        crate::args::BenchCommand::Show { report } => reports::show(report.as_deref()),
    }
}

/// resolves a bare workflow name (e.g. `checkout-settlement`) the same way `kairo run` already
/// does, via `discovery::resolve` -- an existing path or a `.yaml` path is returned unchanged, so
/// this is purely additive for named/bundled demos.
fn resolve_workflow(workflow: &std::path::Path) -> Result<PathBuf, BenchError> {
    Ok(crate::discovery::resolve(
        workflow,
        kairo_core::Config::default(),
    )?)
}

#[allow(clippy::too_many_arguments)]
fn run(
    workflow: &std::path::Path,
    input: Option<&std::path::Path>,
    warmups: u32,
    repetitions: u32,
    output: Option<&std::path::Path>,
    failure_scenario: Option<FailureScenario>,
) -> Result<(), BenchError> {
    let config = config::BenchConfig {
        workflow: workflow.to_path_buf(),
        input: input.map(PathBuf::from),
        warmups,
        repetitions,
        output: output.map(PathBuf::from),
        failure_scenario,
    };
    let path = runner::run(config)?;
    crate::status("32", "✓", &format!("benchmark report · {}", path.display()));
    Ok(())
}
