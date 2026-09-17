use std::{
    collections::{BTreeMap, VecDeque},
    path::Path,
};

use crate::{
    ControlError, RunStatus,
    history::{self, AssignmentReason, RunEvent, RunOutcome},
    server::State,
};

mod live_edge;

impl State {
    pub(crate) fn load(directory: &Path) -> Result<Self, ControlError> {
        let persisted = crate::persistence::load(directory)?;
        let waiting = persisted.waiting;
        let mut live_edges = persisted.live_edges;
        for session in live_edges.values_mut() {
            session.orphan();
        }
        let mut state = Self {
            directory: directory.to_path_buf(),
            workers: BTreeMap::new(),
            queued: VecDeque::new(),
            requests: BTreeMap::new(),
            runs: BTreeMap::new(),
            epochs: BTreeMap::new(),
            waiting,
            live_edges,
            live_assignments: BTreeMap::new(),
            history: BTreeMap::new(),
            pending_reason: BTreeMap::new(),
            dirty: false,
        };
        for (id, record) in persisted.runs {
            state.epochs.insert(id.clone(), record.epoch);
            state.requests.insert(id.clone(), record.request.clone());
            state.history.insert(id.clone(), record.history);
            let status = match record.status {
                RunStatus::Running { .. } => RunStatus::Queued,
                RunStatus::Waiting { reason } if state.waiting.contains_key(&id) => {
                    RunStatus::Waiting { reason }
                }
                RunStatus::Waiting { .. } => RunStatus::Queued,
                // cancellation has nothing left to preserve, unlike `Running`.
                RunStatus::CancelRequested { .. } => RunStatus::Canceled,
                status => status,
            };
            if matches!(status, RunStatus::Queued) && !state.waiting.contains_key(&id) {
                state.queued.push_back(record.request);
                state.record_queued(&id, AssignmentReason::ResumedAfterRestart);
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
                            history: self.history.get(id).cloned().unwrap_or_default(),
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
                live_edges: self.live_edges.clone(),
            },
        )?;
        self.dirty = false;
        Ok(())
    }

    /// records that a run entered the queue, carrying `reason` onto its next assignment.
    pub(crate) fn record_queued(&mut self, id: &str, reason: AssignmentReason) {
        let at_ms = history::now_ms();
        history::push(
            self.history.entry(id.to_owned()).or_default(),
            RunEvent::Queued { at_ms, reason },
        );
        if !matches!(reason, AssignmentReason::Initial) {
            self.pending_reason.insert(id.to_owned(), reason);
        }
        self.dirty = true;
    }

    /// records a real worker/epoch assignment, consuming any pending non-initial reason.
    pub(crate) fn record_assigned(&mut self, id: &str, worker: &str, epoch: u64) {
        let reason = self
            .pending_reason
            .remove(id)
            .unwrap_or(AssignmentReason::Initial);
        let at_ms = history::now_ms();
        history::push(
            self.history.entry(id.to_owned()).or_default(),
            RunEvent::Assigned {
                worker: worker.to_owned(),
                epoch,
                at_ms,
                reason,
            },
        );
        self.dirty = true;
    }

    /// records a run reaching a terminal outcome under a specific worker/epoch.
    pub(crate) fn record_outcome(
        &mut self,
        id: &str,
        worker: &str,
        epoch: u64,
        outcome: RunOutcome,
    ) {
        let at_ms = history::now_ms();
        history::push(
            self.history.entry(id.to_owned()).or_default(),
            RunEvent::Outcome {
                worker: worker.to_owned(),
                epoch,
                at_ms,
                outcome,
            },
        );
        self.dirty = true;
    }

    pub(crate) fn cancel(&mut self, id: &str) -> bool {
        let Some(status) = self.runs.get(id) else {
            return false;
        };
        let canceled = match status {
            RunStatus::Queued | RunStatus::Waiting { .. } => RunStatus::Canceled,
            RunStatus::Running { worker, epoch } => RunStatus::CancelRequested {
                worker: worker.clone(),
                epoch: *epoch,
            },
            RunStatus::CancelRequested { .. } | RunStatus::Canceled => return true,
            RunStatus::Completed { .. } | RunStatus::Failed { .. } => return false,
        };
        self.queued.retain(|run| run.id != id);
        self.invalidate_live_edges_for_run(id, crate::LiveEdgeState::Cancelled);
        self.waiting.remove(id);
        self.runs.insert(id.to_owned(), canceled);
        self.dirty = true;
        true
    }

    /// erases a finished run's record entirely (not just marking it done) -- refused for any run
    /// not already in a terminal state, so an in-flight run can never be silently disappeared.
    pub(crate) fn forget(&mut self, id: &str) -> bool {
        let terminal = matches!(
            self.runs.get(id),
            Some(RunStatus::Completed { .. } | RunStatus::Failed { .. } | RunStatus::Canceled)
        );
        if !terminal {
            return false;
        }
        self.runs.remove(id);
        self.requests.remove(id);
        self.epochs.remove(id);
        self.history.remove(id);
        self.waiting.remove(id);
        self.pending_reason.remove(id);
        self.dirty = true;
        true
    }
}
