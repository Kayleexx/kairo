use serde::Deserialize;

use crate::{ComponentId, Workflow, WorkflowError};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EffectDocument {
    pub(crate) operation: String,
    #[serde(default)]
    pub(crate) after: Option<String>,
}

#[derive(Clone, Debug)]
pub struct WorkflowEffect {
    operation: String,
    after: Option<ComponentId>,
}

impl WorkflowEffect {
    pub fn operation(&self) -> &str {
        &self.operation
    }

    pub fn after(&self) -> Option<&ComponentId> {
        self.after.as_ref()
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
        Some(EffectDocument { operation, after })
            if !operation.is_empty()
                && operation.len() <= 64
                && operation.chars().all(|character| {
                    character.is_ascii_alphanumeric() || matches!(character, '-' | '_')
                }) =>
        {
            Ok(Some(WorkflowEffect {
                operation,
                after: after
                    .map(ComponentId::new)
                    .transpose()
                    .map_err(|_| WorkflowError::InvalidEffect)?,
            }))
        }
        Some(_) => Err(WorkflowError::InvalidEffect),
        None => Ok(None),
    }
}
