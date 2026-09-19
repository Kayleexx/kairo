use kairo_control::{Endpoint, LiveEdgeState, RunOutput, live_edge};

pub(super) fn await_consumer_result(
    executor: &tokio::runtime::Runtime,
    endpoint: &Endpoint,
    session_id: &str,
) -> Result<RunOutput, String> {
    let endpoint = endpoint.clone();
    let session_id = session_id.to_owned();
    executor.block_on(async move {
        tokio::task::spawn_blocking(move || poll_consumer_result(&endpoint, &session_id))
            .await
            .map_err(|_| "live consumer wait task panicked".to_owned())?
    })
}

fn poll_consumer_result(endpoint: &Endpoint, session_id: &str) -> Result<RunOutput, String> {
    for _ in 0..300 {
        let session = live_edge(endpoint, session_id.to_owned())
            .map_err(|error| error.to_string())?
            .ok_or("live edge session disappeared")?;
        if let Some(output) = session.consumer_output {
            return Ok(output);
        }
        if matches!(
            session.state,
            LiveEdgeState::Failed { .. } | LiveEdgeState::Cancelled
        ) {
            return Err("live edge ended before the consumer result arrived".to_owned());
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    Err("timed out waiting for the live consumer result".to_owned())
}
