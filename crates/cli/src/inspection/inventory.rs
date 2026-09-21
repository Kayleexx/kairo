use kairo_runtime::{
    CellInspection, JournalError, StreamRunInspection, ValueRunInspection, inspect_stream_run,
};

use crate::state::{self, LocalCell, StateError};

use super::{InspectionError, live};

pub(super) struct Inventory {
    pub(super) ready: Vec<(LocalCell, CellInspection)>,
    pub(super) streams: Vec<(LocalCell, StreamRunInspection)>,
    pub(super) values: Vec<(LocalCell, ValueRunInspection)>,
    pub(super) unavailable: Vec<(LocalCell, RunReadError)>,
    /// a run tracked by the local control service that hasn't written a journal yet -- e.g. one
    /// still waiting for its very first signal/timer, before any step has run. Without this, such
    /// a run is invisible to `kairo runs` even though `kairo inspect <name>` finds it directly via
    /// the same live control state.
    pub(super) live_only: Vec<kairo_control::RunSnapshot>,
}

pub(super) enum RunReadError {
    Journal(JournalError),
    Stream(kairo_runtime::StreamRunError),
}

impl std::fmt::Display for RunReadError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Journal(error) => error.fmt(formatter),
            Self::Stream(error) => error.fmt(formatter),
        }
    }
}

pub(super) fn load() -> Result<Inventory, StateError> {
    let mut inventory = Inventory {
        ready: Vec::new(),
        streams: Vec::new(),
        values: Vec::new(),
        unavailable: Vec::new(),
        live_only: Vec::new(),
    };
    let mut known = std::collections::HashSet::new();
    for run in state::discover()? {
        known.insert(run.name.clone());
        match inspect_stream_run(&run.path) {
            Ok(Some(inspection)) => inventory.streams.push((run, inspection)),
            Ok(None) => match super::inspect_aggregated_value(&run.path) {
                Ok(Some(inspection)) => inventory.values.push((run, inspection)),
                Ok(None) => match super::inspect_aggregated(&run.path) {
                    Ok(inspection) => inventory.ready.push((run, inspection)),
                    Err(error) => inventory
                        .unavailable
                        .push((run, RunReadError::Journal(error))),
                },
                Err(error) => inventory
                    .unavailable
                    .push((run, RunReadError::Journal(error))),
            },
            Err(error) => inventory
                .unavailable
                .push((run, RunReadError::Stream(error))),
        }
    }
    for run in live::snapshot().unwrap_or_default() {
        if !known.contains(&run.id)
            && !matches!(run.status, kairo_control::RunStatus::Completed { .. })
        {
            inventory.live_only.push(run);
        }
    }
    Ok(inventory)
}

pub(super) fn print_unavailable(inventory: &Inventory) -> Result<(), InspectionError> {
    for (run, error) in &inventory.unavailable {
        let (symbol, state) = if matches!(error, RunReadError::Journal(JournalError::Busy)) {
            (super::marker("36", "●"), "active")
        } else {
            (super::marker("31", "×"), "invalid")
        };
        println!("  {symbol} {} · {state} · {error}", run.name);
    }
    let invalid = inventory
        .unavailable
        .iter()
        .filter(|(_, error)| !matches!(error, RunReadError::Journal(JournalError::Busy)))
        .count();
    if invalid == 0 {
        Ok(())
    } else {
        Err(InspectionError::InvalidRuns { count: invalid })
    }
}
