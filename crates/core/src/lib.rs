use std::fmt;

use thiserror::Error;

pub mod catalog;
mod discovery;
mod durability;
mod effect;
mod grouping;
mod wait;
mod workflow;
mod workflow_model;

pub use discovery::{DiscoveredWorkflow, catalog, discover};
pub use durability::Durability;
pub use effect::WorkflowEffect;
pub use grouping::{GroupSpan, plan_groups};
pub use wait::WorkflowWait;
pub use workflow::{Workflow, WorkflowError};
pub use workflow_model::{
    IoInput, IoOutput, StreamResultLabels, WorkflowEdge, WorkflowInput, WorkflowIo, WorkflowMode,
    WorkflowOutput, WorkflowResources, WorkflowStep,
};

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

impl std::str::FromStr for ComponentHash {
    type Err = ComponentHashError;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        let hex = value
            .strip_prefix("sha256:")
            .ok_or(ComponentHashError::MissingPrefix)?;
        if hex.len() != 64 {
            return Err(ComponentHashError::InvalidLength);
        }
        let mut bytes = [0_u8; 32];
        for (index, byte) in bytes.iter_mut().enumerate() {
            let pair = hex
                .get(index * 2..index * 2 + 2)
                .ok_or(ComponentHashError::InvalidHex)?;
            *byte =
                u8::from_str_radix(pair, 16).map_err(|_source| ComponentHashError::InvalidHex)?;
        }
        Ok(Self(bytes))
    }
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum ComponentHashError {
    #[error("component hash must start with `sha256:`")]
    MissingPrefix,
    #[error("component hash must be 64 hex characters after `sha256:`")]
    InvalidLength,
    #[error("component hash contains non-hex characters")]
    InvalidHex,
}

#[derive(Clone, Copy, Debug)]
pub struct Config {
    pub component_model_async: bool,
    pub allow_console: bool,
    pub max_component_bytes: usize,
    pub max_memory_bytes: usize,
    pub max_workflow_bytes: usize,
    pub max_workflow_steps: usize,
    pub max_stream_input_bytes: u64,
    pub max_stream_output_bytes: u64,
    pub stream_chunk_bytes: usize,
    pub execution_fuel: u64,
    pub max_workflow_fuel: u64,
    pub max_workflow_memory_bytes: usize,
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
            max_stream_input_bytes: 1024 * 1024 * 1024,
            max_stream_output_bytes: 16 * 1024 * 1024,
            stream_chunk_bytes: 64 * 1024,
            execution_fuel: 10_000_000,
            max_workflow_fuel: 5_000_000_000,
            max_workflow_memory_bytes: 256 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum ComponentIdError {
    #[error("component ID cannot be empty")]
    Empty,
}
