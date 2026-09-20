use kairo_control::RunRequest;
use kairo_core::Workflow;

pub(super) fn open_record(
    workflow: &Workflow,
    run: &RunRequest,
) -> Result<kairo_runtime::StreamRun, String> {
    if run.resume.is_some() {
        let replay_child = run
            .plan
            .as_ref()
            .and_then(|plan| plan.replay_source.as_ref())
            .is_some();
        if run.state.exists() || !replay_child {
            return kairo_runtime::StreamRun::open(&run.state).map_err(|error| error.to_string());
        }
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
    let mut record = kairo_runtime::StreamRun::start_with_provenance(
        &run.state,
        workflow.name(),
        &logical,
        source,
        workflow.accepts(),
        &steps,
        labels,
    )
    .map_err(|error| error.to_string())?;
    if let Some(plan) = &run.plan
        && let Some(source) = &plan.replay_source
    {
        let until = plan
            .replay_until
            .and_then(|index| workflow.steps().get(index))
            .ok_or("replay target is outside the workflow")?;
        let boundary = run.resume.as_ref().and_then(|resume| {
            resume
                .from_index
                .checked_sub(1)
                .and_then(|index| workflow.steps().get(index))
                .map(|step| step.id.as_str())
        });
        record
            .record_replay_source(source, until.id.as_str(), boundary)
            .map_err(|error| error.to_string())?;
    }
    Ok(record)
}
