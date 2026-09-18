use std::collections::BTreeMap;

use kairo_control::{
    Assignment, Endpoint, LiveEdgeAssignment, LiveEdgeParticipant, LiveEdgeState, RunOutput,
    RunPlan, RunRequest, WorkerResult, begin_live_edge, complete_live_edge_with_metrics,
    fail_live_edge, live_edge, ready_live_edge, snapshot,
};
use kairo_core::{Durability, Workflow, plan_groups};
use kairo_runtime::{Runtime, StreamGroupInput, StreamGroupOutcome};
use kairo_storage::ArtifactStore;

use crate::live_transport::{EdgeIdentity, LiveEndpoint};

mod cancel;
mod live_consumer;
mod live_record;

pub(crate) use live_consumer::consume_live;

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
    if is_last_group
        && group.end_index > group.start_index
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
    let stop_at = (!is_last_group).then_some(group.end_index);
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
        } => Ok(WorkerResult::Yielded {
            next_index,
            artifact_hash,
            artifact_backend,
            plan: run.plan.is_none().then_some(RunPlan {
                resolved_durability: resolved,
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
    ready_live_edge(
        endpoint,
        self_worker,
        session_id.clone(),
        run.id.clone(),
        edge_id.clone(),
        epoch,
        start_index,
        transport
            .local_addr()
            .map_err(|error| error.to_string())?
            .to_string(),
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
    let relay = executor
        .block_on(async {
            tokio::try_join!(
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
                        .map_err(|error| error.to_string())
                },
                async {
                    transport
                        .serve_relay(identity, source)
                        .await
                        .map_err(|error| error.to_string())
                },
            )
        })
        .map_err(|error| error.to_string());
    let (_, transport_metrics) = match relay {
        Ok(metrics) => metrics,
        Err(message) => {
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
    let output = await_consumer_result(endpoint, &session_id)?;
    let session = live_edge(endpoint, session_id)
        .map_err(|error| error.to_string())?
        .ok_or("live edge session disappeared before recording its result")?;
    live_record::complete(&mut record, &session, &output)?;
    Ok(WorkerResult::Completed(output))
}

fn duration_us(duration: std::time::Duration) -> u64 {
    duration.as_micros().min(u128::from(u64::MAX)) as u64
}

fn await_consumer_result(endpoint: &Endpoint, session_id: &str) -> Result<RunOutput, String> {
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

fn idle_worker(endpoint: &Endpoint, self_worker: &str) -> Result<Option<String>, String> {
    Ok(snapshot(endpoint)
        .map_err(|error| error.to_string())?
        .workers
        .into_iter()
        .filter(|worker| worker.id != self_worker && worker.healthy && !worker.busy)
        .map(|worker| worker.id)
        .next())
}

fn open_record(workflow: &Workflow, run: &RunRequest) -> Result<kairo_runtime::StreamRun, String> {
    if run.resume.is_some() {
        return kairo_runtime::StreamRun::open(&run.state).map_err(|error| error.to_string());
    }
    let input = run
        .stream_input
        .as_deref()
        .or_else(|| workflow.stream_input())
        .ok_or("stream workflow input is required")?;
    let logical = input
        .file_name()
        .unwrap_or(input.as_os_str())
        .to_string_lossy();
    let source = if run.stream_input.is_some() {
        "user"
    } else {
        "bundled"
    };
    let steps = workflow
        .steps()
        .iter()
        .map(|step| step.id.to_string())
        .collect::<Vec<_>>();
    let labels = workflow
        .stream_result_labels()
        .map(|labels| (labels.high.as_str(), labels.low.as_str()));
    kairo_runtime::StreamRun::start_with_provenance(
        &run.state,
        workflow.name(),
        &logical,
        source,
        workflow.accepts(),
        &steps,
        labels,
    )
    .map_err(|error| error.to_string())
}

fn group_input<'a>(
    executor: &tokio::runtime::Runtime,
    workflow: &'a Workflow,
    run: &'a RunRequest,
    artifacts: Option<&ArtifactStore>,
) -> Result<(usize, StreamGroupInput<'a>), String> {
    match &run.resume {
        Some(resume) => {
            let store =
                artifacts.ok_or("resuming a stream execution group needs artifact storage")?;
            let bytes = executor
                .block_on(store.get_bytes(&resume.artifact_hash))
                .map_err(|error| error.to_string())?;
            Ok((resume.from_index, StreamGroupInput::Bytes(bytes)))
        }
        None => {
            let input = run
                .stream_input
                .as_deref()
                .or_else(|| workflow.stream_input())
                .ok_or("stream workflow input is required")?;
            Ok((0, StreamGroupInput::File(input)))
        }
    }
}

fn stream_plan(workflow: &Workflow, run: &RunRequest) -> (BTreeMap<usize, bool>, Option<String>) {
    if let Some(plan) = &run.plan {
        return (plan.resolved_durability.clone(), None);
    }
    let resolved = (0..workflow.steps().len())
        .map(|index| {
            let required = matches!(workflow.durability_after_step(index), Durability::Required);
            (index, required)
        })
        .collect();
    (resolved, None)
}
