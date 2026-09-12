use std::{
    collections::{BTreeMap, VecDeque},
    path::Path,
};

use crate::{ControlError, RunStatus, server::State};

impl State {
    pub(crate) fn load(directory: &Path) -> Result<Self, ControlError> {
        let persisted = crate::persistence::load(directory)?;
        let waiting = persisted.waiting;
        let mut state = Self {
            directory: directory.to_path_buf(),
            workers: BTreeMap::new(),
            queued: VecDeque::new(),
            requests: BTreeMap::new(),
            runs: BTreeMap::new(),
            epochs: BTreeMap::new(),
            waiting,
            dirty: false,
        };
        for (id, record) in persisted.runs {
            state.epochs.insert(id.clone(), record.epoch);
            state.requests.insert(id.clone(), record.request.clone());
            let status = match record.status {
                RunStatus::Running { .. } => RunStatus::Queued,
                RunStatus::Waiting { reason } if state.waiting.contains_key(&id) => {
                    RunStatus::Waiting { reason }
                }
                RunStatus::Waiting { .. } => RunStatus::Queued,
                status => status,
            };
            if matches!(status, RunStatus::Queued) && !state.waiting.contains_key(&id) {
                state.queued.push_back(record.request);
            }
            state.runs.insert(id, status);
        }
        Ok(state)
    }

    pub(crate) fn persist(&mut self) -> Result<(), ControlError> {
        if !self.dirty {
            return Ok(());
        }
        let runs = self
            .requests
            .iter()
            .filter_map(|(id, request)| {
                self.runs.get(id).map(|status| {
                    (
                        id.clone(),
                        crate::persistence::PersistedRun {
                            request: request.clone(),
                            status: status.clone(),
                            epoch: self.epochs.get(id).copied().unwrap_or(0),
                        },
                    )
                })
            })
            .collect();
        crate::persistence::save(
            &self.directory,
            &crate::persistence::PersistedState {
                runs,
                waiting: self.waiting.clone(),
            },
        )?;
        self.dirty = false;
        Ok(())
    }

    pub(crate) fn cancel(&mut self, id: &str) -> bool {
        let Some(status) = self.runs.get(id) else {
            return false;
        };
        let canceled = match status {
            RunStatus::Queued | RunStatus::Waiting { .. } => RunStatus::Canceled,
            RunStatus::Running { .. } => RunStatus::CancelRequested,
            RunStatus::CancelRequested | RunStatus::Canceled => return true,
            RunStatus::Completed { .. } | RunStatus::Failed { .. } => return false,
        };
        self.queued.retain(|run| run.id != id);
        self.waiting.remove(id);
        self.runs.insert(id.to_owned(), canceled);
        self.dirty = true;
        true
    }
}
