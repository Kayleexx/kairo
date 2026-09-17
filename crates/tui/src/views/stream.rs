pub(super) fn live_edges(edges: &[kairo_runtime::StreamLiveEdge]) -> Vec<String> {
    edges
        .iter()
        .map(|edge| {
            format!(
                "observed transport · {}\nworkers · {} → {}\nsent · {} bytes · received · {} bytes\noutcome · {}{}",
                edge.transport,
                edge.producer_worker,
                edge.consumer_worker,
                edge.bytes_sent.unwrap_or(0),
                edge.bytes_received.unwrap_or(0),
                edge.outcome,
                edge.fallback.as_deref().map_or_else(String::new, |fallback| format!("\nfallback · {fallback}")),
            )
        })
        .collect()
}
