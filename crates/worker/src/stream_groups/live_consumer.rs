use kairo_control::{Assignment, Endpoint, LiveEdgeAssignment, WorkerResult, streaming_live_edge};
use kairo_runtime::Runtime;

use crate::live_transport::{EdgeIdentity, LiveEndpoint};

use super::{cancel::Watcher, live_context};

pub(crate) fn consume_live(
    run: kairo_control::RunRequest,
    epoch: u64,
    live: LiveEdgeAssignment,
    allow_console: bool,
    endpoint: &Endpoint,
    self_worker: &str,
) -> Result<WorkerResult, String> {
    let runtime = Runtime::new(kairo_core::Config {
        allow_console,
        ..kairo_core::Config::default()
    })
    .map_err(|error| error.to_string())?;
    let workflow = runtime
        .load_workflow(&run.workflow)
        .map_err(|error| error.to_string())?;
    let executor = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| error.to_string())?;
    let transport =
        executor.block_on(async { LiveEndpoint::bind().map_err(|error| error.to_string()) })?;
    let address = live
        .producer_endpoint
        .parse()
        .map_err(|error| format!("invalid producer endpoint: {error}"))?;
    let (sink, source) = kairo_runtime::StreamRelay::bounded(256 * 1024);
    let _watcher = Watcher::start(
        endpoint.clone(),
        run.id.clone(),
        live.session_id.clone(),
        sink.clone(),
    );
    let identity = EdgeIdentity {
        run_id: &run.id,
        edge_id: &live.edge_id,
        epoch,
    };
    let assignment = Assignment {
        run: run.clone(),
        epoch,
        live_edge: Some(live.clone()),
    };
    let (started_sender, started_receiver) = std::sync::mpsc::sync_channel(1);
    let (ack_sender, ack_receiver) = std::sync::mpsc::sync_channel(1);
    let marker_endpoint = endpoint.clone();
    let marker_worker = self_worker.to_owned();
    let marker = std::thread::spawn(move || {
        started_receiver
            .recv()
            .map_err(|_| "live transport ended before the QUIC handshake".to_owned())?;
        let result = streaming_live_edge(&marker_endpoint, &marker_worker, &assignment)
            .map_err(|error| error.to_string());
        let _ = ack_sender.send(result.clone());
        result
    });
    let metrics_sink = sink.clone();
    let execution = executor.block_on(async {
        tokio::try_join!(
            async {
                transport
                    .fetch_relay_started(
                        address,
                        identity,
                        sink,
                        Some((started_sender, ack_receiver)),
                    )
                    .await
                    .map_err(|error| format!("QUIC receive failed: {error}"))
            },
            async {
                runtime
                    .consume_relay_suffix(&workflow, live.group, source)
                    .await
                    .map_err(|error| format!("consumer component stream failed: {error}"))
            },
        )
    });
    let marked = marker
        .join()
        .map_err(|_| "live QUIC observation thread panicked".to_owned())?;
    marked.map_err(|error| live_context::failure(endpoint, &run.id, &live, "consumer", error))?;
    let (transport_metrics, result) = execution
        .map_err(|error| live_context::failure(endpoint, &run.id, &live, "consumer", error))?;
    let relay = metrics_sink.metrics();
    Ok(WorkerResult::LiveCompleted {
        output: output(result),
        metrics: kairo_control::LiveEdgeMetrics {
            bytes: transport_metrics.bytes,
            duration_us: transport_metrics
                .duration
                .as_micros()
                .min(u128::from(u64::MAX)) as u64,
            peak_buffered_bytes: relay.peak_buffered_bytes,
            first_byte_us: relay.first_produced_us,
        },
    })
}

fn output(result: kairo_runtime::StreamResult) -> kairo_control::RunOutput {
    kairo_control::RunOutput::Stream(kairo_control::StreamOutput {
        bytes: result.bytes,
        checksum: result.checksum,
        values: result
            .values
            .into_iter()
            .map(|value| kairo_control::StreamValue {
                name: value.name,
                value: value.value,
            })
            .collect(),
        outputs: result
            .outputs
            .into_iter()
            .map(|output| kairo_control::StreamArtifact {
                filename: output.filename,
                content_type: output.content_type,
                bytes: output.bytes,
                hash: output.hash,
                backend: output.backend,
                reference: output.reference,
            })
            .collect(),
    })
}
