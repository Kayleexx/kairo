use crate::{RunStatus, server::State};

impl State {
    pub(crate) fn invalidate_live_edges_for_run(
        &mut self,
        run_id: &str,
        state: crate::LiveEdgeState,
    ) {
        let sessions: Vec<String> = self
            .live_edges
            .iter_mut()
            .filter_map(|(id, session)| {
                (session.run_id == run_id
                    && !matches!(
                        session.state,
                        crate::LiveEdgeState::Completed
                            | crate::LiveEdgeState::Failed { .. }
                            | crate::LiveEdgeState::Cancelled
                    ))
                .then(|| {
                    session.state = state.clone();
                    session.finish_observation(Some("run cancelled".to_owned()));
                    id.clone()
                })
            })
            .collect();
        self.live_assignments
            .retain(|_, assignment| !sessions.contains(&assignment.session_id));
    }

    pub(crate) fn invalidate_live_edges_for_worker(&mut self, worker: &str) -> Vec<String> {
        let mut runs = Vec::new();
        let mut participants = Vec::new();
        let sessions: Vec<String> = self
            .live_edges
            .iter_mut()
            .filter_map(|(id, session)| {
                ((session.producer_worker == worker || session.consumer_worker == worker)
                    && !matches!(
                        session.state,
                        crate::LiveEdgeState::Completed
                            | crate::LiveEdgeState::Failed { .. }
                            | crate::LiveEdgeState::Cancelled
                    ))
                .then(|| {
                    runs.push(session.run_id.clone());
                    participants.push(session.producer_worker.clone());
                    participants.push(session.consumer_worker.clone());
                    session.state = crate::LiveEdgeState::Failed {
                        reason: format!("live edge participant `{worker}` lost"),
                    };
                    session.finish_observation(Some("participant lost".to_owned()));
                    id.clone()
                })
            })
            .collect();
        self.live_assignments
            .retain(|_, assignment| !sessions.contains(&assignment.session_id));
        for participant in participants {
            if let Some(item) = self.workers.get_mut(&participant) {
                item.busy = false;
            }
        }
        runs.sort();
        runs.dedup();
        runs
    }
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn begin_live_edge(
        &mut self,
        session_id: String,
        run_id: String,
        edge_id: String,
        parent_epoch: u64,
        producer_group: usize,
        producer_worker: String,
        consumer_group: usize,
        consumer_worker: String,
    ) -> Result<(), String> {
        if self.live_edges.contains_key(&session_id) {
            return Err("live edge session already exists".to_owned());
        }
        if self.epochs.get(&run_id).copied() != Some(parent_epoch) {
            return Err("live edge parent epoch is stale".to_owned());
        }
        if !matches!(self.runs.get(&run_id), Some(RunStatus::Running { worker, epoch }) if worker == &producer_worker && *epoch == parent_epoch)
        {
            return Err("producer does not own the current run epoch".to_owned());
        }
        self.live_edges.insert(
            session_id.clone(),
            crate::LiveEdgeSession::pending(
                session_id,
                run_id,
                edge_id,
                parent_epoch,
                producer_group,
                producer_worker,
                consumer_group,
                consumer_worker,
            ),
        );
        self.dirty = true;
        Ok(())
    }

    pub(crate) fn transition_live_edge(
        &mut self,
        session_id: &str,
        participant: crate::LiveEdgeParticipant,
        worker: &str,
        parent_epoch: u64,
        next: crate::LiveEdgeState,
        endpoint: Option<String>,
    ) -> Result<(), String> {
        if self
            .epochs
            .get(
                self.live_edges
                    .get(session_id)
                    .map_or("", |session| session.run_id.as_str()),
            )
            .copied()
            != Some(parent_epoch)
        {
            return Err("live edge parent epoch is stale".to_owned());
        }
        let session = self
            .live_edges
            .get_mut(session_id)
            .ok_or_else(|| "live edge session is missing".to_owned())?;
        session
            .transition(participant, worker, parent_epoch, next, endpoint)
            .map_err(|error| format!("invalid live edge transition: {error:?}"))?;
        if matches!(session.state, crate::LiveEdgeState::Ready) {
            let consumer = self
                .workers
                .get(&session.consumer_worker)
                .ok_or_else(|| "consumer worker is not registered".to_owned())?;
            if consumer.busy || consumer.last_seen.elapsed() > std::time::Duration::from_secs(3) {
                return Err("consumer worker is not available".to_owned());
            }
            let endpoint = session
                .endpoint
                .clone()
                .ok_or_else(|| "live edge endpoint is missing".to_owned())?;
            self.live_assignments.insert(
                session.consumer_worker.clone(),
                crate::LiveEdgeAssignment {
                    session_id: session.id.clone(),
                    edge_id: session.edge_id.clone(),
                    parent_epoch: session.parent_epoch,
                    producer_endpoint: endpoint,
                    group: session.consumer_group,
                },
            );
        }
        self.dirty = true;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn live_edge_event(
        &mut self,
        session_id: &str,
        run_id: &str,
        edge_id: &str,
        participant: crate::LiveEdgeParticipant,
        group: usize,
        worker: &str,
        parent_epoch: u64,
        next: crate::LiveEdgeState,
        endpoint: Option<String>,
    ) -> Result<(), String> {
        if self.epochs.get(run_id).copied() != Some(parent_epoch) {
            return Err("live edge parent epoch is stale".to_owned());
        }
        let session = self
            .live_edges
            .get(session_id)
            .ok_or_else(|| "live edge session is missing".to_owned())?;
        let expected_group = match participant {
            crate::LiveEdgeParticipant::Producer => session.producer_group,
            crate::LiveEdgeParticipant::Consumer => session.consumer_group,
        };
        if session.run_id != run_id || session.edge_id != edge_id || expected_group != group {
            return Err("live edge identity does not match its session".to_owned());
        }
        self.transition_live_edge(
            session_id,
            participant,
            worker,
            parent_epoch,
            next,
            endpoint,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn complete_live_edge(
        &mut self,
        session_id: &str,
        run_id: &str,
        edge_id: &str,
        participant: crate::LiveEdgeParticipant,
        group: usize,
        worker: &str,
        parent_epoch: u64,
        output: Option<crate::RunOutput>,
        metrics: Option<crate::LiveEdgeMetrics>,
    ) -> Result<(), String> {
        if self.epochs.get(run_id).copied() != Some(parent_epoch) {
            return Err("live edge parent epoch is stale".to_owned());
        }
        let session = self
            .live_edges
            .get_mut(session_id)
            .ok_or_else(|| "live edge session is missing".to_owned())?;
        let expected_group = match participant {
            crate::LiveEdgeParticipant::Producer => session.producer_group,
            crate::LiveEdgeParticipant::Consumer => session.consumer_group,
        };
        if session.run_id != run_id || session.edge_id != edge_id || expected_group != group {
            return Err("live edge identity does not match its session".to_owned());
        }
        session.complete(participant, worker, parent_epoch, output, metrics).map_err(|error| {
            format!(
                "invalid live edge completion: {error:?} (participant={participant:?}, state={:?}, producer_completed={}, consumer_completed={})",
                session.state, session.producer_completed, session.consumer_completed
            )
        })?;
        if matches!(participant, crate::LiveEdgeParticipant::Consumer)
            && let Some(worker) = self.workers.get_mut(worker)
        {
            worker.busy = false;
            worker.last_seen = std::time::Instant::now();
        }
        self.dirty = true;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn fail_live_edge(
        &mut self,
        session_id: &str,
        run_id: &str,
        edge_id: &str,
        participant: crate::LiveEdgeParticipant,
        group: usize,
        worker: &str,
        parent_epoch: u64,
        message: String,
    ) -> Result<(), String> {
        if self.epochs.get(run_id).copied() != Some(parent_epoch) {
            return Err("live edge parent epoch is stale".to_owned());
        }
        let session = self
            .live_edges
            .get(session_id)
            .ok_or_else(|| "live edge session is missing".to_owned())?;
        let expected_group = match participant {
            crate::LiveEdgeParticipant::Producer => session.producer_group,
            crate::LiveEdgeParticipant::Consumer => session.consumer_group,
        };
        if session.run_id != run_id || session.edge_id != edge_id || expected_group != group {
            return Err("live edge identity does not match its session".to_owned());
        }
        if matches!(
            session.state,
            crate::LiveEdgeState::Failed { .. } | crate::LiveEdgeState::Cancelled
        ) {
            if matches!(participant, crate::LiveEdgeParticipant::Consumer)
                && let Some(worker) = self.workers.get_mut(worker)
            {
                worker.busy = false;
                worker.last_seen = std::time::Instant::now();
            }
            return Ok(());
        }
        self.live_edge_event(
            session_id,
            run_id,
            edge_id,
            participant,
            group,
            worker,
            parent_epoch,
            crate::LiveEdgeState::Failed { reason: message },
            None,
        )?;
        self.live_assignments
            .retain(|_, assignment| assignment.session_id != session_id);
        if let Some(session) = self.live_edges.get_mut(session_id) {
            session.finish_observation(Some("live edge failed".to_owned()));
        }
        if matches!(participant, crate::LiveEdgeParticipant::Consumer)
            && let Some(worker) = self.workers.get_mut(worker)
        {
            worker.busy = false;
            worker.last_seen = std::time::Instant::now();
        }
        Ok(())
    }
}
