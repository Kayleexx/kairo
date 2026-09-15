use std::{path::Path, time::Instant};

use kairo_core::{ComponentHash, Workflow, WorkflowResources};
use kairo_storage::ArtifactStore;

use crate::{
    CallKind, Result, Runtime, RuntimeError, StoreState,
    cell::{Cell, PendingStep},
    identity::StepIdentity,
    journal::{Journal, JournalError},
    payload::{EventPayload, INLINE_THRESHOLD_BYTES, LocalBlobStore},
};

mod component {
    wasmtime::component::bindgen!({
        world: "value-stage",
        path: "../../wit",
    });
}

pub(super) struct PreparedValueStep {
    name: String,
    hash: ComponentHash,
    stage: component::ValueStagePre<StoreState>,
}

struct ValueStepResult {
    output: Vec<u8>,
    duration: std::time::Duration,
}

#[derive(Clone, Debug)]
pub struct ValueWorkflowResult {
    pub output: Vec<u8>,
    pub duration: std::time::Duration,
    pub resumed: bool,
}

fn value_step_identities(
    workflow: &Workflow,
    prepared: &[PreparedValueStep],
) -> Result<Vec<StepIdentity>> {
    prepared
        .iter()
        .enumerate()
        .map(|(index, step)| {
            let durable_after = match workflow.durability_after_step(index) {
                kairo_core::Durability::Required => true,
                kairo_core::Durability::Ephemeral => false,
                kairo_core::Durability::Auto => {
                    return Err(RuntimeError::ValueDurabilityAutoUnsupported {
                        step: step.name.clone(),
                    });
                }
            };
            Ok(StepIdentity {
                name: step.name.clone(),
                hash: step.hash,
                durable_after,
            })
        })
        .collect()
}

fn blob_store(state_path: &Path) -> std::result::Result<LocalBlobStore, RuntimeError> {
    LocalBlobStore::open(state_path.with_extension("blobs"))
        .map_err(|source| RuntimeError::LocalBlob { source })
}

fn to_payload(bytes: Vec<u8>, blobs: &LocalBlobStore) -> Result<EventPayload> {
    if bytes.len() <= INLINE_THRESHOLD_BYTES {
        return Ok(EventPayload::Inline(bytes));
    }
    let (hash, bytes) = blobs
        .put(&bytes)
        .map_err(|source| RuntimeError::LocalBlob { source })?;
    Ok(EventPayload::Local { hash, bytes })
}

fn from_payload(payload: EventPayload, blobs: &LocalBlobStore) -> Result<Vec<u8>> {
    match payload {
        EventPayload::Inline(bytes) => Ok(bytes),
        EventPayload::Local { hash, .. } => blobs
            .get(&hash)
            .map_err(|source| RuntimeError::LocalBlob { source }),
        EventPayload::Scalar(_) | EventPayload::Reuse => Err(RuntimeError::Journal {
            path: std::path::PathBuf::new(),
            source: JournalError::InvalidState {
                message: "value cell holds a non-bytes payload".to_owned(),
            },
        }),
    }
}

impl Runtime {
    pub fn validate_value_workflow(&self, workflow: &Workflow) -> Result<()> {
        self.prepare_value_workflow(workflow).map(|_| ())
    }

    pub async fn run_value_workflow(
        &self,
        workflow: &Workflow,
        input: Vec<u8>,
    ) -> Result<ValueWorkflowResult> {
        let prepared = self.prepare_value_workflow(workflow)?;
        let started = Instant::now();
        let mut output = input;
        for step in &prepared {
            output = self
                .run_value_workflow_step(step, output, workflow.resources())
                .await?
                .output;
        }
        let duration = started.elapsed();
        Ok(ValueWorkflowResult {
            output,
            duration,
            resumed: false,
        })
    }

    pub async fn run_value_cell(
        &self,
        workflow: &Workflow,
        state_path: impl AsRef<Path>,
        artifacts: Option<&ArtifactStore>,
        input: Vec<u8>,
    ) -> Result<ValueWorkflowResult> {
        let state_path = state_path.as_ref();
        let prepared = self.prepare_value_workflow(workflow)?;
        let identities = value_step_identities(workflow, &prepared)?;
        let blobs = blob_store(state_path)?;

        let journal =
            Journal::open(state_path).map_err(|source| self.journal_error(state_path, source))?;
        let input_payload = to_payload(input, &blobs)?;
        let mut cell = Cell::open(
            journal,
            workflow.name(),
            input_payload.clone(),
            &identities,
            &[],
            0,
            input_payload,
        )
        .map_err(|source| self.journal_error(state_path, source))?;
        let resumed = cell.resumed();

        let started = Instant::now();
        while let Some(pending) = cell.next(prepared.len()) {
            let (index, state_payload, input) = match pending {
                PendingStep::Local { index, input } => {
                    let bytes = from_payload(input.clone(), &blobs)?;
                    (index, input, bytes)
                }
                PendingStep::Checkpoint {
                    index,
                    input,
                    hash,
                    backend,
                } => {
                    let artifacts = artifacts.ok_or(RuntimeError::ArtifactStoreRequired)?;
                    let configured = artifacts.backend().as_str();
                    if let Some(recorded) =
                        backend.clone().filter(|recorded| recorded != configured)
                    {
                        return Err(RuntimeError::CheckpointBackendMismatch {
                            hash,
                            recorded,
                            configured,
                        });
                    }
                    let bytes = artifacts
                        .get_bytes(&hash)
                        .await
                        .map_err(|source| RuntimeError::Artifact { source })?;
                    tracing::info!(index, hash, "checkpoint restored");
                    (index, input, bytes)
                }
            };
            let (step, identity) =
                prepared
                    .get(index)
                    .zip(identities.get(index))
                    .ok_or_else(|| {
                        self.journal_error(
                            state_path,
                            JournalError::InvalidState {
                                message: "next component is out of bounds".to_owned(),
                            },
                        )
                    })?;
            cell.start(index, identity, state_payload)
                .map_err(|source| self.journal_error(state_path, source))?;
            tracing::info!(
                step = step.name,
                index,
                input_bytes = input.len(),
                "cell component started"
            );
            let result = self
                .run_value_workflow_step(step, input, workflow.resources())
                .await?;
            // ponytail: a large durable_after output still pays for a local blob write here in
            // addition to the durable checkpoint below -- correct, but duplicates storage for
            // that one combination. Upgrade: check `identity.durable_after` before choosing to
            // write the local blob at all.
            let output_payload = to_payload(result.output.clone(), &blobs)?;
            cell.complete_component(
                index,
                output_payload,
                super::duration_us(result.duration),
                identity.durable_after,
            )
            .map_err(|source| self.journal_error(state_path, source))?;
            if identity.durable_after {
                let store = artifacts.ok_or(RuntimeError::ArtifactStoreRequired)?;
                let checkpoint_started = std::time::Instant::now();
                let mut writer = store
                    .begin_bytes(self.config.max_stream_output_bytes)
                    .await
                    .map_err(|source| RuntimeError::Artifact { source })?;
                writer
                    .write(&result.output)
                    .map_err(|source| RuntimeError::Artifact { source })?;
                let artifact = writer
                    .finish()
                    .await
                    .map_err(|source| RuntimeError::Artifact { source })?;
                cell.checkpoint(
                    index,
                    artifact.hash.clone(),
                    store.backend().as_str().to_owned(),
                    artifact.bytes,
                    super::duration_us(checkpoint_started.elapsed()),
                )
                .map_err(|source| self.journal_error(state_path, source))?;
                tracing::info!(index, hash = artifact.hash, "checkpoint created");
            }
        }
        let output_payload = cell
            .finish(prepared.len())
            .map_err(|source| self.journal_error(state_path, source))?;
        let output = from_payload(output_payload, &blobs)?;
        let duration = started.elapsed();
        tracing::info!(
            workflow = workflow.name(),
            duration_us = duration.as_micros(),
            output_bytes = output.len(),
            resumed,
            state = %state_path.display(),
            "value cell executed"
        );
        Ok(ValueWorkflowResult {
            output,
            duration,
            resumed,
        })
    }

    fn prepare_value_workflow(&self, workflow: &Workflow) -> Result<Vec<PreparedValueStep>> {
        self.validate_workflow_resources(workflow)?;
        let linker = self.component_linker()?;
        let mut prepared = Vec::with_capacity(workflow.steps().len());
        for step in workflow.steps() {
            let loaded = self
                .load_component(&step.component)
                .map_err(|source| self.workflow_step_error(step.id.as_str(), source))?;
            let pre = linker
                .instantiate_pre(&loaded.component)
                .map_err(|source| {
                    self.workflow_step_error(step.id.as_str(), RuntimeError::Instantiate { source })
                })?;
            let stage = component::ValueStagePre::new(pre).map_err(|source| {
                RuntimeError::IncompatibleWorkflowComponent {
                    step: step.id.to_string(),
                    path: step.component.clone(),
                    source,
                }
            })?;
            prepared.push(PreparedValueStep {
                name: step.id.to_string(),
                hash: loaded.hash,
                stage,
            });
        }
        Ok(prepared)
    }

    async fn run_value_workflow_step(
        &self,
        step: &PreparedValueStep,
        input: Vec<u8>,
        resources: Option<WorkflowResources>,
    ) -> Result<ValueStepResult> {
        let mut store = self.new_store(resources)?;
        let stage = step
            .stage
            .instantiate_async(&mut store)
            .await
            .map_err(|source| {
                self.workflow_step_error(&step.name, self.instantiation_error(source, &store))
            })?;
        let started = Instant::now();
        let output = match store
            .run_concurrent(async |accessor| stage.call_run(accessor, input).await)
            .await
        {
            Ok(Ok(Ok(output))) => output,
            Ok(Ok(Err(message))) => {
                return Err(self.workflow_step_error(
                    &step.name,
                    RuntimeError::ValueStepRejected {
                        step: step.name.clone(),
                        message,
                    },
                ));
            }
            Ok(Err(source)) => {
                let error = self.execution_error(source, &store, CallKind::Workflow);
                return Err(self.workflow_step_error(&step.name, error));
            }
            Err(source) => {
                let error = self.execution_error(source, &store, CallKind::Runtime);
                return Err(self.workflow_step_error(&step.name, error));
            }
        };
        let duration = started.elapsed();
        tracing::info!(
            step = step.name,
            hash = %step.hash,
            duration_us = duration.as_micros(),
            output_bytes = output.len(),
            "value workflow step executed"
        );
        Ok(ValueStepResult { output, duration })
    }
}
