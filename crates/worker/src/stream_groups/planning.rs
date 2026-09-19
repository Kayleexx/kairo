use std::collections::BTreeMap;

use kairo_control::{RunOutput, RunRequest};
use kairo_core::{Durability, Workflow};
use kairo_runtime::StreamGroupInput;
use kairo_storage::ArtifactStore;

pub(super) fn group_input<'a>(
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

pub(super) fn stream_plan(
    workflow: &Workflow,
    run: &RunRequest,
) -> (BTreeMap<usize, bool>, Option<String>) {
    if let Some(plan) = &run.plan {
        let resolved = (0..workflow.steps().len())
            .map(|index| {
                (
                    index,
                    plan.resolved_durability
                        .get(&index)
                        .copied()
                        .unwrap_or(matches!(
                            workflow.durability_after_step(index),
                            Durability::Required
                        )),
                )
            })
            .collect();
        return (resolved, None);
    }
    let resolved = (0..workflow.steps().len())
        .map(|index| {
            let required = matches!(workflow.durability_after_step(index), Durability::Required);
            (index, required)
        })
        .collect();
    (resolved, None)
}

pub(super) fn output_reference(output: &RunOutput) -> String {
    let RunOutput::Stream(stream) = output else {
        return String::new();
    };
    stream
        .outputs
        .first()
        .map_or_else(String::new, |artifact| artifact.reference.clone())
}
