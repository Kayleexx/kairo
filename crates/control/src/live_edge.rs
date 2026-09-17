use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub struct LiveEdgeMetrics {
    pub bytes: u64,
    pub duration_us: u64,
    pub peak_buffered_bytes: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub struct LiveEdgeObservation {
    pub transport: String,
    pub started_at_ms: u64,
    #[serde(default)]
    pub ended_at_ms: Option<u64>,
    #[serde(default)]
    pub producer: Option<LiveEdgeMetrics>,
    #[serde(default)]
    pub consumer: Option<LiveEdgeMetrics>,
    #[serde(default)]
    pub fallback: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub struct LiveEdgeSession {
    pub id: String,
    pub run_id: String,
    pub edge_id: String,
    pub parent_epoch: u64,
    pub producer_group: usize,
    pub producer_worker: String,
    pub consumer_group: usize,
    pub consumer_worker: String,
    pub state: LiveEdgeState,
    #[serde(default)]
    pub endpoint: Option<String>,
    #[serde(default)]
    pub producer_completed: bool,
    #[serde(default)]
    pub consumer_completed: bool,
    #[serde(default)]
    pub consumer_output: Option<crate::RunOutput>,
    #[serde(default)]
    pub observation: Option<LiveEdgeObservation>,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub enum LiveEdgeState {
    Pending,
    Assigned,
    Ready,
    Streaming,
    Completed,
    Failed { reason: String },
    Cancelled,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub enum LiveEdgeParticipant {
    Producer,
    Consumer,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LiveEdgeTransitionError {
    StaleEpoch,
    WrongWorker,
    IllegalState,
    MissingEndpoint,
}

impl LiveEdgeSession {
    #[allow(clippy::too_many_arguments)]
    pub fn pending(
        id: String,
        run_id: String,
        edge_id: String,
        parent_epoch: u64,
        producer_group: usize,
        producer_worker: String,
        consumer_group: usize,
        consumer_worker: String,
    ) -> Self {
        Self {
            id,
            run_id,
            edge_id,
            parent_epoch,
            producer_group,
            producer_worker,
            consumer_group,
            consumer_worker,
            state: LiveEdgeState::Pending,
            endpoint: None,
            producer_completed: false,
            consumer_completed: false,
            consumer_output: None,
            observation: None,
        }
    }

    pub fn complete(
        &mut self,
        participant: LiveEdgeParticipant,
        worker: &str,
        parent_epoch: u64,
        output: Option<crate::RunOutput>,
        metrics: Option<LiveEdgeMetrics>,
    ) -> Result<(), LiveEdgeTransitionError> {
        self.validate(participant, worker, parent_epoch)?;
        let completed = match participant {
            LiveEdgeParticipant::Producer => &mut self.producer_completed,
            LiveEdgeParticipant::Consumer => &mut self.consumer_completed,
        };
        let participant_may_complete = match participant {
            LiveEdgeParticipant::Producer => {
                matches!(self.state, LiveEdgeState::Ready | LiveEdgeState::Streaming)
            }
            LiveEdgeParticipant::Consumer => matches!(self.state, LiveEdgeState::Streaming),
        };
        if *completed
            || !participant_may_complete
            || matches!(participant, LiveEdgeParticipant::Consumer) && output.is_none()
        {
            return Err(LiveEdgeTransitionError::IllegalState);
        }
        *completed = true;
        if let Some(metrics) = metrics {
            let observation = self.observation.get_or_insert_with(observation);
            match participant {
                LiveEdgeParticipant::Producer => observation.producer = Some(metrics),
                LiveEdgeParticipant::Consumer => observation.consumer = Some(metrics),
            }
        }
        if matches!(participant, LiveEdgeParticipant::Consumer) {
            self.consumer_output = output;
        }
        if self.producer_completed && self.consumer_completed {
            self.state = LiveEdgeState::Completed;
            if let Some(observation) = &mut self.observation {
                observation.ended_at_ms = Some(now_ms());
            }
        }
        Ok(())
    }

    fn validate(
        &self,
        participant: LiveEdgeParticipant,
        worker: &str,
        parent_epoch: u64,
    ) -> Result<(), LiveEdgeTransitionError> {
        if self.parent_epoch != parent_epoch {
            return Err(LiveEdgeTransitionError::StaleEpoch);
        }
        let expected = match participant {
            LiveEdgeParticipant::Producer => &self.producer_worker,
            LiveEdgeParticipant::Consumer => &self.consumer_worker,
        };
        (expected == worker)
            .then_some(())
            .ok_or(LiveEdgeTransitionError::WrongWorker)
    }

    pub fn transition(
        &mut self,
        participant: LiveEdgeParticipant,
        worker: &str,
        parent_epoch: u64,
        next: LiveEdgeState,
        endpoint: Option<String>,
    ) -> Result<(), LiveEdgeTransitionError> {
        self.validate(participant, worker, parent_epoch)?;
        if !legal(&self.state, &next) {
            return Err(LiveEdgeTransitionError::IllegalState);
        }
        if matches!(next, LiveEdgeState::Ready) && endpoint.is_none() {
            return Err(LiveEdgeTransitionError::MissingEndpoint);
        }
        if endpoint.is_some() {
            self.endpoint = endpoint;
        }
        if matches!(next, LiveEdgeState::Streaming) {
            self.observation.get_or_insert_with(observation);
        }
        self.state = next;
        Ok(())
    }

    pub fn orphan(&mut self) {
        if !terminal(&self.state) {
            self.state = LiveEdgeState::Failed {
                reason: "control plane restarted during live edge".to_owned(),
            };
            self.finish_observation(Some("control plane restart".to_owned()));
        }
    }

    pub fn finish_observation(&mut self, fallback: Option<String>) {
        if let Some(observation) = &mut self.observation {
            observation.ended_at_ms = Some(now_ms());
            observation.fallback = fallback;
        }
    }
}

fn observation() -> LiveEdgeObservation {
    LiveEdgeObservation {
        transport: "remote-live".to_owned(),
        started_at_ms: now_ms(),
        ended_at_ms: None,
        producer: None,
        consumer: None,
        fallback: None,
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            duration.as_millis().min(u128::from(u64::MAX)) as u64
        })
}

fn legal(current: &LiveEdgeState, next: &LiveEdgeState) -> bool {
    !terminal(current)
        && matches!(
            (current, next),
            (LiveEdgeState::Pending, LiveEdgeState::Assigned)
                | (LiveEdgeState::Assigned, LiveEdgeState::Ready)
                | (LiveEdgeState::Ready, LiveEdgeState::Streaming)
                | (_, LiveEdgeState::Failed { .. })
                | (_, LiveEdgeState::Cancelled)
        )
}

fn terminal(state: &LiveEdgeState) -> bool {
    matches!(
        state,
        LiveEdgeState::Completed | LiveEdgeState::Failed { .. } | LiveEdgeState::Cancelled
    )
}
