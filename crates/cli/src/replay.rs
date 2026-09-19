use std::path::Path;

use kairo_control::{GroupResume, ReplaySource, SubmissionOutcome};
use kairo_core::Config;
use kairo_runtime::Runtime;
use kairo_storage::ArtifactStore;

use crate::{CliError, Result, service, state, status};

pub(crate) async fn run(source_id: &str, until: &str, config: Config, verbose: bool) -> Result<()> {
    let source = kairo_control::replay_source(Path::new(".kairo"), source_id)?;
    let runtime = Runtime::new(config)?;
    let workflow = runtime.load_workflow(&source.request.workflow)?;
    let until_index = workflow
        .steps()
        .iter()
        .position(|step| step.id.as_str() == until)
        .ok_or_else(|| rejected(format!("step `{until}` is not in the source workflow")))?;
    let boundary = reusable_boundary(&source, until_index).await?;
    let child_state = state::generated_run(&format!("{}-replay", workflow.name()))?;
    let (mut request, lineage) = kairo_control::prepare_replay(
        Path::new(".kairo"),
        source_id,
        until,
        child_state,
        boundary,
    )?;
    if request.storage.is_none() && until_index + 1 < workflow.steps().len() {
        let _ = crate::setup::ensure_storage()?;
        let mut storage = crate::setup::storage_config()?;
        if storage.local {
            storage.endpoint = std::env::current_dir()
                .map_err(|source| {
                    CliError::Control(kairo_control::ControlError::ResolvePath { source })
                })?
                .join(&storage.endpoint)
                .to_string_lossy()
                .into_owned();
        }
        request.storage = Some(storage);
    }
    let child_id = request.id.clone();
    let (endpoint, mut local) = service::ensure_endpoint(None, config.allow_console, verbose)?;
    kairo_control::submit_with_lineage(&endpoint, request, Some(lineage))?;
    status("36", "→", &format!("replaying {source_id} through {until}"));
    let outcome = kairo_control::await_run(&endpoint, &child_id, false)?;
    if let Some(local) = &mut local {
        local.stop()?;
    }
    match outcome {
        SubmissionOutcome::Completed(output) => {
            status("32", "✓", &format!("replay completed {child_id}"));
            println!("replay · {child_id} · source {source_id} · through {until}");
            println!("{output}");
            Ok(())
        }
        SubmissionOutcome::Canceled | SubmissionOutcome::Waiting { .. } => {
            Err(rejected("replay did not reach a completed result"))
        }
    }
}

async fn reusable_boundary(
    source: &ReplaySource,
    until_index: usize,
) -> Result<Option<GroupResume>> {
    let candidates = source
        .lineage
        .boundaries
        .iter()
        .filter(|boundary| boundary.from_index <= until_index)
        .rev();
    let Some(storage) = &source.request.storage else {
        return Ok(None);
    };
    let store = ArtifactStore::from_config(storage.clone())?;
    for boundary in candidates {
        if boundary.artifact_backend != store.backend().as_str() {
            continue;
        }
        if store.get_bytes(&boundary.artifact_hash).await.is_ok() {
            return Ok(Some(boundary.clone()));
        }
    }
    Ok(None)
}

fn rejected(message: impl Into<String>) -> CliError {
    CliError::Control(kairo_control::ControlError::Rejected {
        message: message.into(),
    })
}
