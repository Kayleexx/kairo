use std::time::Duration;

use serde::Deserialize;

use crate::WorkflowError;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WaitDocument {
    pub(crate) timer_ms: Option<u64>,
    pub(crate) signal: Option<String>,
}

#[derive(Clone, Debug)]
pub enum WorkflowWait {
    Timer(Duration),
    Signal(String),
}

pub(crate) fn parse_wait(
    value: Option<WaitDocument>,
) -> Result<Option<WorkflowWait>, WorkflowError> {
    match value {
        Some(WaitDocument {
            timer_ms: Some(milliseconds),
            signal: None,
        }) => Ok(Some(WorkflowWait::Timer(Duration::from_millis(
            milliseconds,
        )))),
        Some(WaitDocument {
            timer_ms: None,
            signal: Some(signal),
        }) if !signal.is_empty()
            && signal.len() <= 128
            && !signal.chars().any(char::is_control) =>
        {
            Ok(Some(WorkflowWait::Signal(signal)))
        }
        Some(_) => Err(WorkflowError::InvalidWait),
        None => Ok(None),
    }
}
