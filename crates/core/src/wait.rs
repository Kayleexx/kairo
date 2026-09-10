use std::time::Duration;

use serde::Deserialize;

use crate::{ComponentId, Workflow, WorkflowError};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WaitDocument {
    pub(crate) timer_ms: Option<u64>,
    pub(crate) signal: Option<String>,
    #[serde(default)]
    pub(crate) after: Option<String>,
}

#[derive(Clone, Debug)]
pub enum WorkflowWait {
    Timer(Duration),
    Signal(String),
}

impl Workflow {
    pub fn wait(&self) -> Option<&WorkflowWait> {
        self.wait.as_ref()
    }

    pub fn wait_after(&self) -> Option<&ComponentId> {
        self.wait_after.as_ref()
    }
}

pub(crate) fn parse_wait(
    value: Option<WaitDocument>,
) -> Result<(Option<WorkflowWait>, Option<ComponentId>), WorkflowError> {
    match value {
        Some(WaitDocument {
            timer_ms: Some(milliseconds),
            signal: None,
            after,
        }) => Ok((
            Some(WorkflowWait::Timer(Duration::from_millis(milliseconds))),
            component(after)?,
        )),
        Some(WaitDocument {
            timer_ms: None,
            signal: Some(name),
            after,
        }) if !name.is_empty() && name.len() <= 128 && !name.chars().any(char::is_control) => {
            Ok((Some(WorkflowWait::Signal(name)), component(after)?))
        }
        Some(_) => Err(WorkflowError::InvalidWait),
        None => Ok((None, None)),
    }
}

fn component(value: Option<String>) -> Result<Option<ComponentId>, WorkflowError> {
    value
        .map(ComponentId::new)
        .transpose()
        .map_err(|_| WorkflowError::InvalidWait)
}
