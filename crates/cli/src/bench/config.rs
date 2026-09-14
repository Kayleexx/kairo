use std::path::PathBuf;

pub(crate) struct BenchConfig {
    pub(crate) workflow: PathBuf,
    pub(crate) input: Option<PathBuf>,
    pub(crate) warmups: u32,
    pub(crate) repetitions: u32,
    pub(crate) output: Option<PathBuf>,
    pub(crate) failure_scenario: Option<FailureScenario>,
}

#[derive(Clone, Copy)]
pub(crate) enum FailureScenario {
    WorkerKill,
}
