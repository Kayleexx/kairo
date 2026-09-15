use std::{collections::HashMap, path::Path};

use kairo_core::{Durability, Workflow};

use super::PreparedStep;
use crate::{
    Result, Runtime, RuntimeError,
    durability_plan::{self, AutoResolution},
    identity,
};

impl Runtime {
    /// profile key for `.kairo/profiles/<shape>.json`.
    pub fn workflow_shape(&self, workflow: &Workflow) -> Result<String> {
        let prepared = self.prepare_workflow(workflow)?;
        Ok(identity::workflow_shape(
            workflow.name(),
            &prepared
                .iter()
                .map(|step| (step.name.clone(), step.hash))
                .collect::<Vec<_>>(),
        ))
    }

    pub fn auto_edge_profile(&self, workflow: &Workflow, index: usize) -> Result<Option<String>> {
        let prepared = self.prepare_workflow(workflow)?;
        let shape = self.workflow_shape(workflow)?;
        let step = &prepared[index];
        let Some(profile) = durability_plan::load_profile(&shape)
            .and_then(|workflow_profile| workflow_profile.edges.get(&step.name).copied())
        else {
            return Ok(None);
        };
        let (required, reason) = durability_plan::decide(&profile);
        let label = if required { "required" } else { "ephemeral" };
        Ok(Some(format!("{label} · {reason}")))
    }

    // resumed runs reuse the journaled decision (`peek_plan`); fresh runs resolve from a real,
    // previously measured profile and return the resolutions for `Cell::open` to journal.
    pub(super) fn resolve_durability(
        &self,
        workflow: &Workflow,
        prepared: &[PreparedStep],
        state_path: &Path,
    ) -> Result<(HashMap<usize, bool>, Vec<AutoResolution>)> {
        let auto_indices: Vec<usize> = (0..prepared.len())
            .filter(|&index| workflow.durability_after_step(index) == Durability::Auto)
            .collect();
        if auto_indices.is_empty() {
            return Ok((HashMap::new(), Vec::new()));
        }
        let peeked = durability_plan::peek_plan(state_path)
            .map_err(|source| self.journal_error(state_path, source))?;
        let shape = identity::workflow_shape(
            workflow.name(),
            &prepared
                .iter()
                .map(|step| (step.name.clone(), step.hash))
                .collect::<Vec<_>>(),
        );
        let mut resolved = HashMap::new();
        let mut auto_plan = Vec::new();
        for index in auto_indices {
            if let Some(required) = peeked.as_ref().and_then(|plan| plan.get(&index)).copied() {
                resolved.insert(index, required);
                continue;
            }
            let step = &prepared[index];
            let profile = durability_plan::load_profile(&shape)
                .and_then(|workflow_profile| workflow_profile.edges.get(&step.name).copied())
                .ok_or_else(|| RuntimeError::DurabilityProfileMissing {
                    step: step.name.clone(),
                })?;
            let (required, reason) = durability_plan::decide(&profile);
            resolved.insert(index, required);
            auto_plan.push(AutoResolution {
                index,
                required,
                profile_id: shape.clone(),
                reason,
            });
        }
        Ok((resolved, auto_plan))
    }
}
