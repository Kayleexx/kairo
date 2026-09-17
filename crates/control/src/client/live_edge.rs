use crate::{
    Assignment, ControlError, Endpoint, LiveEdgeAssignment, LiveEdgeParticipant, Request, Response,
    RunOutput,
};

use super::{ok, request};

#[allow(clippy::too_many_arguments)]
pub fn begin_live_edge(
    endpoint: &Endpoint,
    worker: &str,
    session_id: String,
    id: String,
    edge_id: String,
    epoch: u64,
    producer_group: usize,
    consumer_group: usize,
    consumer_worker: String,
) -> Result<(), ControlError> {
    ok(request(
        endpoint,
        Request::LiveEdgeBegin {
            worker: worker.to_owned(),
            token: endpoint.token.clone(),
            session_id,
            id,
            edge_id,
            epoch,
            producer_group,
            consumer_group,
            consumer_worker,
        },
    )?)
}

#[allow(clippy::too_many_arguments)]
pub fn ready_live_edge(
    endpoint: &Endpoint,
    worker: &str,
    session_id: String,
    id: String,
    edge_id: String,
    epoch: u64,
    producer_group: usize,
    endpoint_metadata: String,
) -> Result<(), ControlError> {
    ok(request(
        endpoint,
        Request::LiveEdgeReady {
            worker: worker.to_owned(),
            token: endpoint.token.clone(),
            session_id,
            id,
            edge_id,
            epoch,
            producer_group,
            endpoint: endpoint_metadata,
        },
    )?)
}

pub fn streaming_live_edge(
    endpoint: &Endpoint,
    worker: &str,
    assignment: &Assignment,
) -> Result<(), ControlError> {
    let live = assignment.live_edge.as_ref().ok_or(ControlError::State)?;
    ok(request(
        endpoint,
        Request::LiveEdgeStreaming {
            worker: worker.to_owned(),
            token: endpoint.token.clone(),
            session_id: live.session_id.clone(),
            id: assignment.run.id.clone(),
            edge_id: live.edge_id.clone(),
            epoch: assignment.epoch,
            consumer_group: live.group,
        },
    )?)
}

pub fn complete_live_edge(
    endpoint: &Endpoint,
    worker: &str,
    assignment: &Assignment,
    participant: LiveEdgeParticipant,
) -> Result<(), ControlError> {
    complete_live_edge_with_metrics(endpoint, worker, assignment, participant, None, None)
}

pub fn complete_live_edge_output(
    endpoint: &Endpoint,
    worker: &str,
    assignment: &Assignment,
    participant: LiveEdgeParticipant,
    output: Option<RunOutput>,
) -> Result<(), ControlError> {
    complete_live_edge_with_metrics(endpoint, worker, assignment, participant, output, None)
}

pub fn complete_live_edge_with_metrics(
    endpoint: &Endpoint,
    worker: &str,
    assignment: &Assignment,
    participant: LiveEdgeParticipant,
    output: Option<RunOutput>,
    metrics: Option<crate::LiveEdgeMetrics>,
) -> Result<(), ControlError> {
    let (session_id, edge_id, group) = live_identity(assignment)?;
    ok(request(
        endpoint,
        Request::LiveEdgeComplete {
            worker: worker.to_owned(),
            token: endpoint.token.clone(),
            session_id,
            id: assignment.run.id.clone(),
            edge_id,
            epoch: assignment.epoch,
            group,
            participant,
            output,
            metrics,
        },
    )?)
}

pub fn live_edge(
    endpoint: &Endpoint,
    session_id: String,
) -> Result<Option<crate::LiveEdgeSession>, ControlError> {
    match request(
        endpoint,
        Request::LiveEdgeStatus {
            token: endpoint.token.clone(),
            session_id,
        },
    )? {
        Response::LiveEdge { session } => Ok(*session),
        Response::Error { message } => Err(ControlError::Rejected { message }),
        _ => Err(ControlError::State),
    }
}

pub fn fail_live_edge(
    endpoint: &Endpoint,
    worker: &str,
    assignment: &Assignment,
    participant: LiveEdgeParticipant,
    message: String,
) -> Result<(), ControlError> {
    let (session_id, edge_id, group) = live_identity(assignment)?;
    ok(request(
        endpoint,
        Request::LiveEdgeFail {
            worker: worker.to_owned(),
            token: endpoint.token.clone(),
            session_id,
            id: assignment.run.id.clone(),
            edge_id,
            epoch: assignment.epoch,
            group,
            participant,
            message,
        },
    )?)
}

fn live_identity(assignment: &Assignment) -> Result<(String, String, usize), ControlError> {
    let live: &LiveEdgeAssignment = assignment.live_edge.as_ref().ok_or(ControlError::State)?;
    Ok((live.session_id.clone(), live.edge_id.clone(), live.group))
}
