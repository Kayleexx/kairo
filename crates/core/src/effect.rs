use serde::Deserialize;

use crate::{Workflow, WorkflowError};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EffectDocument {
    pub(crate) operation: String,
}

#[derive(Clone, Debug)]
pub struct WorkflowEffect {
    operation: String,
}

impl WorkflowEffect {
    pub fn operation(&self) -> &str {
        &self.operation
    }
}

impl Workflow {
    pub fn effect(&self) -> Option<&WorkflowEffect> {
        self.effect.as_ref()
    }
}

pub(crate) fn parse_effect(
    value: Option<EffectDocument>,
) -> Result<Option<WorkflowEffect>, WorkflowError> {
    match value {
        Some(EffectDocument { operation })
            if !operation.is_empty()
                && operation.len() <= 64
                && operation.chars().all(|character| {
                    character.is_ascii_alphanumeric() || matches!(character, '-' | '_')
                }) =>
        {
            Ok(Some(WorkflowEffect { operation }))
        }
        Some(_) => Err(WorkflowError::InvalidEffect),
        None => Ok(None),
    }
}
