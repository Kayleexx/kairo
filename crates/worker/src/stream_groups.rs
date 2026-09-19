use kairo_control::{
    Assignment, Endpoint, LiveEdgeAssignment, LiveEdgeParticipant, RunOutput, RunPlan, RunRequest,
    WorkerResult, begin_live_edge, complete_live_edge_with_metrics, fail_live_edge, live_edge,
    ready_live_edge, snapshot,
};
use kairo_core::{Workflow, plan_groups};
use kairo_runtime::{Runtime, StreamGroupInput, StreamGroupOutcome};
use kairo_storage::ArtifactStore;

use crate::live_transport::{EdgeIdentity, LiveEndpoint};

mod cancel;
mod live_completion;
mod live_consumer;
mod live_context;
mod live_record;
mod planning;
mod record;

pub(crate) use live_consumer::consume_live;
use planning::{group_input, output_reference, stream_plan};
use record::open_record;

#[allow(clippy::too_many_arguments)]
pub(crate) fn execute(
    executor: &tokio::runtime::Runtime,
    runtime: &Runtime,
    workflow: &Workflow,
    run: &RunRequest,
    artifacts: Option<&ArtifactStore>,
    endpoint: &Endpoint,
    self_worker: &str,
    epoch: u64,
) -> Result<WorkerResult, String> {
    let (resolved, shape) = stream_plan(workflow, run);
    let groups = plan_groups(workflow, &resolved);
    let (start_index, input) = group_input(executor, workflow, run, artifacts)?;
    let group = groups
        .iter()
        .find(|group| group.start_index == start_index)
        .ok_or("stream execution group boundary mismatch")?;
    let is_last_group = group.end_index + 1 == workflow.steps().len();
    let replay_until = run.plan.as_ref().and_then(|plan| plan.replay_until);
    if is_last_group
        && group.end_index > group.start_index
        && replay_until.is_none_or(|until| until >= group.end_index)
        && let Some(consumer) = idle_worker(endpoint, self_worker)?
    {
        return relay_prefix(
            executor,
            runtime,
            workflow,
            run,
            input,
            group.start_index,
            endpoint,
            self_worker,
            &consumer,
            epoch,
        );
    }
    let mut record = open_record(workflow, run)?;
    let stop_at = replay_until
        .filter(|until| *until >= group.start_index && *until < workflow.steps().len() - 1)
        .or_else(|| (!is_last_group).then_some(group.end_index));
    let outcome = match executor.block_on(runtime.run_stream_cell_group(
        workflow,
        artifacts,
        start_index,
        input,
        stop_at,
    )) {
        Ok(outcome) => outcome,
        Err(error) => {
            let message = error.to_string();
            if let Err(record_error) = record.fail(&message) {
                tracing::warn!(%record_error, "failed to record stream group failure");
            }
            return Err(message);
        }
    };
    match outcome {
        StreamGroupOutcome::Completed(result) => {
            record
                .complete_with_outputs(
                    result.duration,
                    result.bytes,
                    result.checksum,
                    result.metrics.clone(),
                    (!result.input_hash.is_empty()).then_some(result.input_hash.as_str()),
                    &result.values,
                    &result.outputs,
                )
                .map_err(|error| error.to_string())?;
            Ok(WorkerResult::Completed(kairo_control::RunOutput::Stream(
                kairo_control::StreamOutput {
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
                },
            )))
        }
        StreamGroupOutcome::Yielded {
            next_index,
            artifact_hash,
            artifact_backend,
            bytes,
            ..
        } if replay_until == next_index.checked_sub(1) => {
            let step = workflow
                .steps()
                .get(next_index - 1)
                .ok_or("replay target is outside the workflow")?;
            let output = RunOutput::Stream(kairo_control::StreamOutput {
                bytes,
                checksum: 0,
                values: Vec::new(),
                outputs: vec![kairo_control::StreamArtifact {
                    filename: format!("replay-{}.bin", step.id),
                    content_type: "application/octet-stream".to_owned(),
                    bytes,
                    hash: artifact_hash.clone(),
                    backend: artifact_backend.clone(),
                    reference: format!("artifacts/{artifact_hash}"),
                }],
            });
            record
                .complete_with_outputs(
                    std::time::Duration::ZERO,
                    bytes,
                    0,
                    kairo_runtime::StreamMetrics::default(),
                    None,
                    &[],
                    &[kairo_runtime::WorkflowOutputArtifact {
                        filename: format!("replay-{}.bin", step.id),
                        content_type: "application/octet-stream".to_owned(),
                        bytes,
                        hash: artifact_hash,
                        backend: artifact_backend,
                        reference: output_reference(&output),
                        exported_path: None,
                    }],
                )
                .map_err(|error| error.to_string())?;
            Ok(WorkerResult::ReplayCompleted(output))
        }
        StreamGroupOutcome::Yielded {
            next_index,
            artifact_hash,
            artifact_backend,
            bytes: _,
            ..
        } => Ok(WorkerResult::Yielded {
            next_index,
            artifact_hash,
            artifact_backend,
            plan: run.plan.is_none().then_some(RunPlan {
                resolved_durability: resolved,
                replay_until: None,
                replay_source: None,
            }),
            shape,
            target_worker: None,
            target_had_cache: false,
        }),
    }
}

#[allow(clippy::too_many_arguments)]
fn relay_prefix(
    executor: &tokio::runtime::Runtime,
    runtime: &Runtime,
    workflow: &Workflow,
    run: &RunRequest,
    input: StreamGroupInput<'_>,
    start_index: usize,
    endpoint: &Endpoint,
    self_worker: &str,
    consumer: &str,
    epoch: u64,
) -> Result<WorkerResult, String> {
    let transport =
        executor.block_on(async { LiveEndpoint::bind().map_err(|error| error.to_string()) })?;
    let consumer_start = start_index + 1;
    let edge_id = format!(
        "{}-to-{}",
        workflow.steps()[start_index].id,
        workflow.steps()[consumer_start].id
    );
    let session_id = format!("{}:{edge_id}:{epoch}", run.id);
    let mut record = open_record(workflow, run)?;
    begin_live_edge(
        endpoint,
        self_worker,
        session_id.clone(),
        run.id.clone(),
        edge_id.clone(),
        epoch,
        start_index,
        consumer_start,
        consumer.to_owned(),
    )
    .map_err(|error| error.to_string())?;
    let (sink, source) = kairo_runtime::StreamRelay::bounded(256 * 1024);
    let _watcher = cancel::Watcher::start(
        endpoint.clone(),
        run.id.clone(),
        session_id.clone(),
        sink.clone(),
    );
    let metrics_source = source.clone();
    let identity = EdgeIdentity {
        run_id: &run.id,
        edge_id: &edge_id,
        epoch,
    };
    let producer_assignment = Assignment {
        run: run.clone(),
        epoch,
        live_edge: Some(LiveEdgeAssignment {
            session_id: session_id.clone(),
            edge_id: edge_id.clone(),
            parent_epoch: epoch,
            producer_endpoint: String::new(),
            group: start_index,
        }),
    };
    let ready_endpoint = endpoint.clone();
    let ready_worker = self_worker.to_owned();
    let ready_session = session_id.clone();
    let ready_run = run.id.clone();
    let ready_edge = edge_id.clone();
    let ready_address = transport
        .local_addr()
        .map_err(|error| error.to_string())?
        .to_string();
    let (accepting, accepted) = tokio::sync::oneshot::channel();
    let relay = executor
        .block_on(async {
            tokio::try_join!(
                async move {
                    accepted
                        .await
                        .map_err(|_| "live edge accept task ended before readiness".to_owned())?;
                    tokio::task::spawn_blocking(move || {
                        ready_live_edge(
                            &ready_endpoint,
                            &ready_worker,
                            ready_session,
                            ready_run,
                            ready_edge,
                            epoch,
                            start_index,
                            ready_address,
                        )
                        .map_err(|error| error.to_string())
                    })
                    .await
                    .map_err(|_| "live edge readiness task panicked".to_owned())?
                },
                async {
                    runtime
                        .relay_stream_group_prefix(
                            workflow,
                            start_index,
                            input,
                            consumer_start,
                            sink,
                        )
                        .await
                        .map_err(|error| format!("producer component relay failed: {error}"))
                },
                async {
                    transport
                        .serve_relay_started(identity, source, Some(accepting))
                        .await
                        .map_err(|error| format!("QUIC send failed: {error}"))
                },
            )
        })
        .map_err(|error| error.to_string());
    let (_, _, transport_metrics) = match relay {
        Ok(metrics) => metrics,
        Err(message) => {
            let message = live_context::failure(
                endpoint,
                &run.id,
                producer_assignment
                    .live_edge
                    .as_ref()
                    .ok_or("missing live edge")?,
                "producer",
                message,
            );
            let _ = record.fail(&message);
            let _ = fail_live_edge(
                endpoint,
                self_worker,
                &producer_assignment,
                LiveEdgeParticipant::Producer,
                message.clone(),
            );
            return Err(message);
        }
    };
    complete_live_edge_with_metrics(
        endpoint,
        self_worker,
        &producer_assignment,
        LiveEdgeParticipant::Producer,
        None,
        Some(kairo_control::LiveEdgeMetrics {
            bytes: transport_metrics.bytes,
            duration_us: transport_metrics
                .duration
                .as_micros()
                .min(u128::from(u64::MAX)) as u64,
            peak_buffered_bytes: metrics_source.metrics().peak_buffered_bytes,
            first_byte_us: transport_metrics.first_byte.map(duration_us),
        }),
    )
    .map_err(|error| error.to_string())?;
    let output = live_completion::await_consumer_result(executor, endpoint, &session_id)?;
    drop(transport_metrics);
    let session = live_edge(endpoint, session_id)
        .map_err(|error| error.to_string())?
        .ok_or("live edge session disappeared before recording its result")?;
    live_record::complete(&mut record, &session, &output)?;
    Ok(WorkerResult::Completed(output))
}

fn duration_us(duration: std::time::Duration) -> u64 {
    duration.as_micros().min(u128::from(u64::MAX)) as u64
}

fn idle_worker(endpoint: &Endpoint, self_worker: &str) -> Result<Option<String>, String> {
    Ok(snapshot(endpoint)
        .map_err(|error| error.to_string())?
        .workers
        .into_iter()
        .filter(|worker| worker.id != self_worker && worker.healthy && !worker.busy)
        .map(|worker| worker.id)
        .next())
}
