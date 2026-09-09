use std::{collections::BTreeMap, fs, path::Path};

use serde::{Deserialize, Serialize};

use crate::WaitRequest;
use crate::{ControlError, RunRequest, RunStatus};

#[derive(Default, Deserialize, Serialize)]
pub(crate) struct PersistedState {
    pub(crate) runs: BTreeMap<String, PersistedRun>,
    #[serde(default)]
    pub(crate) waiting: BTreeMap<String, WaitRequest>,
}

#[derive(Deserialize, Serialize)]
pub(crate) struct PersistedRun {
    pub(crate) request: RunRequest,
    pub(crate) status: RunStatus,
    pub(crate) epoch: u64,
}

pub(crate) fn load(directory: &Path) -> Result<PersistedState, ControlError> {
    let path = directory.join("control-state.json");
    match fs::read(path) {
        Ok(bytes) => {
            serde_json::from_slice(&bytes).map_err(|source| ControlError::Protocol { source })
        }
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            Ok(PersistedState::default())
        }
        Err(source) => Err(ControlError::Io { source }),
    }
}

pub(crate) fn save(directory: &Path, state: &PersistedState) -> Result<(), ControlError> {
    let path = directory.join("control-state.json");
    let temporary = directory.join("control-state.next");
    let bytes = serde_json::to_vec(state).map_err(|source| ControlError::Protocol { source })?;
    fs::write(&temporary, bytes).map_err(|source| ControlError::Io { source })?;
    fs::rename(temporary, path).map_err(|source| ControlError::Io { source })
}
