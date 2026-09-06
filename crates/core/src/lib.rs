use std::fmt;

use thiserror::Error;

mod workflow;

pub use workflow::{Workflow, WorkflowEdge, WorkflowError, WorkflowStep};

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ComponentId(String);

impl ComponentId {
    pub fn new(value: impl Into<String>) -> std::result::Result<Self, ComponentIdError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(ComponentIdError::Empty);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ComponentId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ComponentHash([u8; 32]);

impl ComponentHash {
    pub fn sha256(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
}

impl fmt::Display for ComponentHash {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("sha256:")?;
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Config {
    pub component_model_async: bool,
    pub allow_console: bool,
    pub max_component_bytes: usize,
    pub max_memory_bytes: usize,
    pub max_workflow_bytes: usize,
    pub max_workflow_steps: usize,
    pub execution_fuel: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            component_model_async: true,
            allow_console: false,
            max_component_bytes: 64 * 1024 * 1024,
            max_memory_bytes: 64 * 1024 * 1024,
            max_workflow_bytes: 1024 * 1024,
            max_workflow_steps: 256,
            execution_fuel: 10_000_000,
        }
    }
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum ComponentIdError {
    #[error("component ID cannot be empty")]
    Empty,
}
