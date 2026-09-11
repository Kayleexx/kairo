use std::path::PathBuf;

use crate::{ComponentId, Durability};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkflowMode {
    Scalar,
    Stream,
}

#[derive(Clone, Debug)]
pub enum WorkflowInput {
    Scalar(u32),
    File(PathBuf),
}

#[derive(Clone, Debug)]
pub struct WorkflowStep {
    pub id: ComponentId,
    pub component: PathBuf,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WorkflowResources {
    pub fuel: u64,
    pub memory_bytes: usize,
}

#[derive(Clone, Debug)]
pub struct WorkflowEdge {
    pub from: ComponentId,
    pub to: ComponentId,
    pub durability: Durability,
}

#[derive(Clone, Debug)]
pub struct StreamResultLabels {
    pub high: String,
    pub low: String,
}
