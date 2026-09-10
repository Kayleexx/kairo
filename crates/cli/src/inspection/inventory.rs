use kairo_runtime::{
    CellInspection, JournalError, StreamRunInspection, inspect_cell, inspect_stream_run,
};

use crate::state::{self, LocalCell, StateError};

use super::InspectionError;

pub(super) struct Inventory {
    pub(super) ready: Vec<(LocalCell, CellInspection)>,
    pub(super) streams: Vec<(LocalCell, StreamRunInspection)>,
    pub(super) unavailable: Vec<(LocalCell, RunReadError)>,
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
        unavailable: Vec::new(),
    };
    for run in state::discover()? {
        match inspect_stream_run(&run.path) {
            Ok(Some(inspection)) => inventory.streams.push((run, inspection)),
            Ok(None) => match inspect_cell(&run.path) {
                Ok(inspection) => inventory.ready.push((run, inspection)),
                Err(error) => inventory
                    .unavailable
                    .push((run, RunReadError::Journal(error))),
            },
            Err(error) => inventory
                .unavailable
                .push((run, RunReadError::Stream(error))),
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
