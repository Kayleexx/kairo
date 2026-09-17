use std::path::PathBuf;

use crate::{ComponentHash, ComponentId, Durability};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkflowMode {
    Scalar,
    Stream,
    Value,
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
    /// pinned at compose time from the component's real content hash; when present, the runtime
    /// refuses to run if the loaded component no longer matches it.
    pub pinned_hash: Option<ComponentHash>,
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

#[derive(Clone, Debug)]
pub struct WorkflowOutput {
    pub filename: String,
    pub content_type: String,
}

/// UX/contract metadata only -- never parsed or enforced against a component's actual behavior.
/// Drives a guided CLI/TUI prompt: whether to ask for input at all, and for what shape.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum IoInput {
    #[default]
    None,
    File,
    Value,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum IoOutput {
    #[default]
    None,
    Value,
    Artifact,
}

#[derive(Clone, Debug, Default)]
pub struct WorkflowIo {
    pub input: IoInput,
    pub output: IoOutput,
    pub filename: Option<String>,
}
