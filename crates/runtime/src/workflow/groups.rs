use std::{collections::BTreeMap, path::Path};

use kairo_core::Workflow;
use kairo_storage::ArtifactStore;

use crate::{Result, Runtime, durability_plan::AutoResolution};

use super::CoreOutcome;

#[derive(Clone, Debug)]
pub enum GroupOutcome {
    Completed {
        output: u32,
    },
    Yielded {
        next_index: usize,
        artifact_hash: String,
        artifact_backend: String,
    },
}

impl Runtime {
    /// Runs one ExecutionGroup: the workflow's steps from `start_index` up to and including
    /// `stop_at` (or to the workflow's actual end when `stop_at` is `None`), starting from
    /// `start_input` instead of the workflow's own declared input. `resolved_durability` and
    /// `auto_plan` must come from an already-computed `RunPlan` -- this never re-resolves
    /// `durability: auto` edges itself, so every group in a run agrees on the same decision.
    #[allow(clippy::too_many_arguments)]
    pub async fn run_cell_group(
        &self,
        workflow: &Workflow,
        state_path: impl AsRef<Path>,
        artifacts: Option<&ArtifactStore>,
        resolved_durability: &BTreeMap<usize, bool>,
        auto_plan: &[AutoResolution],
        start_index: usize,
        start_input: u32,
        stop_at: Option<usize>,
    ) -> Result<GroupOutcome> {
        let prepared = self.prepare_workflow(workflow)?;
        let fingerprint_input = workflow
            .scalar_input()
            .ok_or(crate::RuntimeError::InvalidScalarWorkflowInput)?;
        let resolved: std::collections::HashMap<usize, bool> = resolved_durability
            .iter()
            .map(|(&index, &required)| (index, required))
            .collect();
        let identities = super::step_identities(workflow, &prepared, &resolved);
        let state_path = state_path.as_ref();
        match self
            .run_cell_core(
                workflow,
                &prepared,
                &identities,
                auto_plan,
                state_path,
                artifacts,
                fingerprint_input,
                start_index,
                start_input,
                stop_at,
            )
            .await?
        {
            CoreOutcome::Completed(result) => Ok(GroupOutcome::Completed {
                output: result.output,
            }),
            CoreOutcome::Paused { hash, backend, .. } => Ok(GroupOutcome::Yielded {
                next_index: stop_at
                    .and_then(|index| index.checked_add(1))
                    .unwrap_or(prepared.len()),
                artifact_hash: hash,
                artifact_backend: backend,
            }),
        }
    }
}
