use std::time::Duration;

use kairo_control::{LiveEdgeSession, RunOutput};
use kairo_runtime::{
    StreamLiveEdge, StreamMetrics, StreamRun, StreamValue, WorkflowOutputArtifact,
};

pub(super) fn complete(
    record: &mut StreamRun,
    session: &LiveEdgeSession,
    output: &RunOutput,
) -> Result<(), String> {
    let RunOutput::Stream(output) = output else {
        return Err("live stream consumer returned a scalar output".to_owned());
    };
    let observed = session
        .observation
        .as_ref()
        .ok_or("live stream completed without an observed QUIC session")?;
    record
        .record_live_edge(&StreamLiveEdge {
            edge_id: session.edge_id.clone(),
            transport: observed.transport.clone(),
            producer_worker: session.producer_worker.clone(),
            consumer_worker: session.consumer_worker.clone(),
            parent_epoch: session.parent_epoch,
            bytes_sent: observed.producer.as_ref().map(|metrics| metrics.bytes),
            bytes_received: observed.consumer.as_ref().map(|metrics| metrics.bytes),
            started_at_ms: observed.started_at_ms,
            ended_at_ms: observed.ended_at_ms,
            outcome: "completed".to_owned(),
            fallback: observed.fallback.clone(),
        })
        .map_err(|error| error.to_string())?;
    record
        .complete_with_outputs(
            Duration::from_micros(
                observed
                    .producer
                    .as_ref()
                    .map_or(0, |metrics| metrics.duration_us),
            ),
            output.bytes,
            output.checksum,
            StreamMetrics {
                source_bytes: observed
                    .producer
                    .as_ref()
                    .map_or(0, |metrics| metrics.bytes),
                consumed_bytes: observed
                    .consumer
                    .as_ref()
                    .map_or(0, |metrics| metrics.bytes),
                ..StreamMetrics::default()
            },
            None,
            &output
                .values
                .iter()
                .map(|value| StreamValue {
                    name: value.name.clone(),
                    value: value.value,
                })
                .collect::<Vec<_>>(),
            &output
                .outputs
                .iter()
                .map(|artifact| WorkflowOutputArtifact {
                    filename: artifact.filename.clone(),
                    content_type: artifact.content_type.clone(),
                    bytes: artifact.bytes,
                    hash: artifact.hash.clone(),
                    backend: artifact.backend.clone(),
                    reference: artifact.reference.clone(),
                    exported_path: None,
                })
                .collect::<Vec<_>>(),
        )
        .map_err(|error| error.to_string())
}
