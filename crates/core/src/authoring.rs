use std::path::PathBuf;

use serde::Serialize;
use thiserror::Error;

use crate::{ComponentHash, Durability, WorkflowMode};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum ComponentRole {
    ValueStage,
    ScalarStage,
    StreamTransform,
    StreamConsume,
    StreamConsumeMetrics,
    StreamOutput,
}

impl ComponentRole {
    pub fn mode(self) -> WorkflowMode {
        match self {
            Self::ValueStage => WorkflowMode::Value,
            Self::ScalarStage => WorkflowMode::Scalar,
            Self::StreamTransform
            | Self::StreamConsume
            | Self::StreamConsumeMetrics
            | Self::StreamOutput => WorkflowMode::Stream,
        }
    }

    pub fn shape(self) -> &'static str {
        match self {
            Self::ValueStage => "value → value",
            Self::ScalarStage => "number → number",
            Self::StreamTransform => "byte stream → byte stream",
            Self::StreamConsume => "byte stream → number",
            Self::StreamConsumeMetrics => "byte stream → value",
            Self::StreamOutput => "byte stream → artifact",
        }
    }

    pub fn can_follow(self, previous: Option<Self>) -> bool {
        match previous {
            None => true,
            Some(Self::ValueStage) => self == Self::ValueStage,
            Some(Self::ScalarStage) => self == Self::ScalarStage,
            Some(Self::StreamTransform) => matches!(
                self,
                Self::StreamTransform
                    | Self::StreamConsume
                    | Self::StreamConsumeMetrics
                    | Self::StreamOutput
            ),
            Some(_) => false,
        }
    }

    pub fn finishable(self, step_count: usize) -> bool {
        self != Self::StreamTransform
            && (self.mode() != WorkflowMode::Stream
                || self == Self::StreamOutput
                || step_count >= 2)
    }

    pub fn can_continue(self) -> bool {
        matches!(
            self,
            Self::ValueStage | Self::ScalarStage | Self::StreamTransform
        )
    }
}

#[derive(Clone, Debug)]
pub struct DraftStep {
    pub name: String,
    pub component: PathBuf,
    pub hash: Option<ComponentHash>,
}

#[derive(Clone, Debug)]
pub enum DraftWait {
    Timer(u64),
    Signal(String),
}

#[derive(Clone, Debug)]
pub struct WorkflowDraft {
    pub name: String,
    pub description: Option<String>,
    pub accepts: Vec<String>,
    pub produces: Vec<String>,
    pub mode: WorkflowMode,
    pub scalar_input: u32,
    pub steps: Vec<DraftStep>,
    pub durabilities: Vec<Durability>,
    pub output: Option<(String, String)>,
    pub wait: Option<DraftWait>,
    pub effect: Option<String>,
}

#[derive(Debug, Error)]
pub enum AuthoringError {
    #[error("workflow needs at least one Component")]
    NoSteps,
    #[error("workflow has {steps} steps but {edges} edge durability values")]
    EdgeCount { steps: usize, edges: usize },
    #[error("failed to serialize workflow")]
    Serialize(#[source] yaml_serde::Error),
}

impl WorkflowDraft {
    pub fn to_yaml(&self) -> Result<String, AuthoringError> {
        if self.steps.is_empty() {
            return Err(AuthoringError::NoSteps);
        }
        let expected_edges = self.steps.len().saturating_sub(1);
        if self.durabilities.len() != expected_edges {
            return Err(AuthoringError::EdgeCount {
                steps: self.steps.len(),
                edges: self.durabilities.len(),
            });
        }
        let io = match self.mode {
            WorkflowMode::Scalar => None,
            WorkflowMode::Value => Some(IoDocument {
                input: "value",
                output: "value",
            }),
            WorkflowMode::Stream => Some(IoDocument {
                input: "file",
                output: if self.output.is_some() {
                    "artifact"
                } else {
                    "value"
                },
            }),
        };
        let document = DraftDocument {
            workflow: &self.name,
            description: self.description.as_deref(),
            accepts: &self.accepts,
            produces: &self.produces,
            mode: mode_name(self.mode),
            input: (self.mode == WorkflowMode::Scalar).then_some(self.scalar_input),
            io,
            steps: self
                .steps
                .iter()
                .map(|step| StepDocument {
                    name: &step.name,
                    component: &step.component,
                    hash: step.hash.map(|hash| hash.to_string()),
                })
                .collect(),
            edges: self
                .steps
                .windows(2)
                .zip(&self.durabilities)
                .map(|(steps, durability)| EdgeDocument {
                    from: &steps[0].name,
                    to: &steps[1].name,
                    durability: durability_name(*durability),
                })
                .collect(),
            output: self
                .output
                .as_ref()
                .map(|(filename, content_type)| OutputDocument {
                    filename,
                    content_type,
                }),
            wait: self.wait.as_ref().map(|wait| match wait {
                DraftWait::Timer(timer_ms) => WaitDocument {
                    timer_ms: Some(*timer_ms),
                    signal: None,
                },
                DraftWait::Signal(signal) => WaitDocument {
                    timer_ms: None,
                    signal: Some(signal),
                },
            }),
            effect: self
                .effect
                .as_ref()
                .map(|operation| EffectDocument { operation }),
        };
        yaml_serde::to_string(&document).map_err(AuthoringError::Serialize)
    }
}

fn mode_name(mode: WorkflowMode) -> &'static str {
    match mode {
        WorkflowMode::Scalar => "scalar",
        WorkflowMode::Stream => "stream",
        WorkflowMode::Value => "value",
    }
}

fn durability_name(durability: Durability) -> &'static str {
    match durability {
        Durability::Ephemeral => "ephemeral",
        Durability::Required => "required",
        Durability::Auto => "auto",
    }
}

#[derive(Serialize)]
struct DraftDocument<'a> {
    workflow: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<&'a str>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    accepts: &'a Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    produces: &'a Vec<String>,
    mode: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    input: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    io: Option<IoDocument>,
    steps: Vec<StepDocument<'a>>,
    edges: Vec<EdgeDocument<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    output: Option<OutputDocument<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    wait: Option<WaitDocument<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    effect: Option<EffectDocument<'a>>,
}

#[derive(Serialize)]
struct IoDocument {
    input: &'static str,
    output: &'static str,
}

#[derive(Serialize)]
struct StepDocument<'a> {
    name: &'a str,
    component: &'a PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    hash: Option<String>,
}

#[derive(Serialize)]
struct EdgeDocument<'a> {
    from: &'a str,
    to: &'a str,
    durability: &'static str,
}

#[derive(Serialize)]
struct OutputDocument<'a> {
    filename: &'a str,
    content_type: &'a str,
}

#[derive(Serialize)]
struct WaitDocument<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    timer_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    signal: Option<&'a String>,
}

#[derive(Serialize)]
struct EffectDocument<'a> {
    operation: &'a String,
}
