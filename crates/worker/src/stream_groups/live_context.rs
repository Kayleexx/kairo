use kairo_control::{Endpoint, LiveEdgeAssignment, live_edge};

pub(super) fn failure(
    endpoint: &Endpoint,
    run_id: &str,
    live: &LiveEdgeAssignment,
    participant: &str,
    error: impl std::fmt::Display,
) -> String {
    let (state, producer_worker, consumer_worker) = live_edge(endpoint, live.session_id.clone())
        .ok()
        .flatten()
        .map(|session| {
            (
                format!("{:?}", session.state),
                session.producer_worker,
                session.consumer_worker,
            )
        })
        .unwrap_or_else(|| {
            (
                "unavailable".to_owned(),
                "unavailable".to_owned(),
                "unavailable".to_owned(),
            )
        });
    format!(
        "{participant} live-edge failure: {error} (run_id={run_id}, session_id={}, edge_id={}, parent_epoch={}, producer_worker={producer_worker}, consumer_worker={consumer_worker}, state={state})",
        live.session_id, live.edge_id, live.parent_epoch
    )
}
