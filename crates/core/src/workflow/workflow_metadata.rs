use std::collections::HashSet;

use super::WorkflowError;

const MAX_ALIASES: usize = 16;
const MAX_ACCEPTS: usize = 16;
const MAX_PRODUCES: usize = 16;

pub(super) fn validate_aliases(name: &str, aliases: &[String]) -> Result<(), WorkflowError> {
    if aliases.len() > MAX_ALIASES || !unique_labels(aliases, 64, Some(name)) {
        return Err(WorkflowError::InvalidAliases);
    }
    Ok(())
}

pub(super) fn validate_accepts(accepts: &[String]) -> Result<(), WorkflowError> {
    if accepts.len() > MAX_ACCEPTS
        || !unique_labels(accepts, 32, None)
        || accepts.iter().any(|value| {
            !value.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '-' | '_')
            })
        })
    {
        return Err(WorkflowError::InvalidAccepts);
    }
    Ok(())
}

pub(super) fn validate_produces(produces: &[String]) -> Result<(), WorkflowError> {
    if produces.len() > MAX_PRODUCES
        || !unique_labels(produces, 32, None)
        || produces.iter().any(|value| {
            !value.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '-' | '_')
            })
        })
    {
        return Err(WorkflowError::InvalidProduces);
    }
    Ok(())
}

pub(super) fn valid_label(value: &str) -> bool {
    !value.is_empty() && value.len() <= 64 && !value.chars().any(char::is_control)
}

fn unique_labels(values: &[String], max_len: usize, reserved: Option<&str>) -> bool {
    let mut seen = HashSet::with_capacity(values.len());
    values.iter().all(|value| {
        !value.is_empty()
            && value.len() <= max_len
            && !value.chars().any(char::is_control)
            && reserved != Some(value.as_str())
            && seen.insert(value)
    })
}
