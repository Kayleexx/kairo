use std::{
    collections::HashMap,
    fs::File,
    io::{self, Read},
    path::{Path, PathBuf},
};

use thiserror::Error;

use crate::effect::{WorkflowEffect, parse_effect};
use crate::wait::{WorkflowWait, parse_wait};
use crate::{
    ComponentId, Durability, StreamResultLabels, WorkflowEdge, WorkflowInput, WorkflowMode,
    WorkflowStep,
};

pub(crate) use self::workflow_document::{DurabilityDocument, EdgeDocument};
use self::workflow_document::{InputDocument, WorkflowDocument, WorkflowModeDocument};
use self::workflow_graph::{validate_boundaries, validate_edges};
use self::workflow_metadata::{valid_label, validate_accepts, validate_aliases};

mod workflow_document;
mod workflow_graph;
mod workflow_metadata;

#[derive(Clone, Debug)]
pub struct Workflow {
    name: String,
    description: Option<String>,
    aliases: Vec<String>,
    accepts: Vec<String>,
    input: Option<WorkflowInput>,
    mode: WorkflowMode,
    steps: Vec<WorkflowStep>,
    pub(crate) edges: Vec<WorkflowEdge>,
    pub(crate) wait: Option<WorkflowWait>,
    pub(crate) wait_after: Option<ComponentId>,
    pub(crate) effect: Option<WorkflowEffect>,
    stream_result: Option<StreamResultLabels>,
}

#[derive(Debug, Error)]
pub enum WorkflowError {
    #[error("failed to open workflow `{path}`")]
    Open {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to read workflow `{path}`")]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("workflow `{path}` exceeds the {max_bytes}-byte size limit")]
    TooLarge { path: PathBuf, max_bytes: usize },
    #[error("workflow YAML is invalid")]
    InvalidYaml {
        #[source]
        source: yaml_serde::Error,
    },
    #[error("workflow is not valid UTF-8")]
    InvalidUtf8 {
        #[source]
        source: std::string::FromUtf8Error,
    },
    #[error("workflow name cannot be empty")]
    EmptyName,
    #[error("workflow description must be at most 200 printable characters")]
    InvalidDescription,
    #[error("workflow aliases must be unique, printable names of at most 64 characters")]
    InvalidAliases,
    #[error(
        "workflow accepted input labels must be unique printable names of at most 32 characters"
    )]
    InvalidAccepts,
    #[error("workflow must contain at least one step")]
    NoSteps,
    #[error("workflow contains {steps} steps, exceeding the {max_steps}-step limit")]
    TooManySteps { steps: usize, max_steps: usize },
    #[error("workflow step {index} has an invalid name")]
    InvalidStepName { index: usize },
    #[error("workflow contains duplicate step `{step}`")]
    DuplicateStep { step: String },
    #[error("workflow step `{step}` has an empty component path")]
    EmptyComponentPath { step: String },
    #[error("workflow edge references unknown step `{step}`")]
    UnknownStep { step: String },
    #[error("workflow edge {index} has an empty `{endpoint}` step")]
    EmptyEdgeStep {
        index: usize,
        endpoint: &'static str,
    },
    #[error("workflow contains duplicate edge `{from}` → `{to}`")]
    DuplicateEdge { from: String, to: String },
    #[error("workflow graph contains a cycle")]
    Cycle,
    #[error("workflow graph is disconnected")]
    Disconnected,
    #[error("step `{step}` has multiple inputs; only linear workflows are supported")]
    MultipleInputs { step: String },
    #[error("step `{step}` has multiple outputs; only linear workflows are supported")]
    MultipleOutputs { step: String },
    #[error("scalar workflow input must be an unsigned integer")]
    ScalarInput,
    #[error("stream workflow input must be a file path")]
    StreamInput,
    #[error("stream workflows require at least one transform and one consumer")]
    StreamWorkflowSteps,
    #[error("stream workflows do not support `durability: required`")]
    StreamDurability,
    #[error("workflow wait must specify exactly one of `timer_ms` or `signal`")]
    InvalidWait,
    #[error("workflow effect operation must be 1–64 letters, digits, `-`, or `_`")]
    InvalidEffect,
    #[error("workflow {kind} references unknown step `{step}`")]
    UnknownBoundaryStep { kind: &'static str, step: String },
    #[error("workflow {kind} after `{step}` requires a durable outgoing edge")]
    InvalidBoundary { kind: &'static str, step: String },
    #[error("workflow wait and effect cannot use the same boundary")]
    ConflictingBoundaries,
    #[error("stream workflows do not support waits or effects")]
    StreamControl,
    #[error("stream result labels must be 1–64 printable characters")]
    InvalidStreamResult,
    #[error("scalar workflows do not support stream result labels")]
    ScalarStreamResult,
}

impl Workflow {
    pub fn load(
        path: impl AsRef<Path>,
        max_bytes: usize,
        max_steps: usize,
    ) -> Result<Self, WorkflowError> {
        let path = path.as_ref();
        let source = read_bounded(path, max_bytes)?;
        let base = path.parent().unwrap_or_else(|| Path::new("."));
        Self::parse(&source, base, max_steps)
    }

    pub fn parse(source: &str, base: &Path, max_steps: usize) -> Result<Self, WorkflowError> {
        let document: WorkflowDocument =
            yaml_serde::from_str(source).map_err(|source| WorkflowError::InvalidYaml { source })?;
        if document.workflow.trim().is_empty() {
            return Err(WorkflowError::EmptyName);
        }
        if document.description.as_ref().is_some_and(|description| {
            description.chars().count() > 200 || description.chars().any(char::is_control)
        }) {
            return Err(WorkflowError::InvalidDescription);
        }
        validate_aliases(&document.workflow, &document.aliases)?;
        validate_accepts(&document.accepts)?;
        if document.steps.is_empty() {
            return Err(WorkflowError::NoSteps);
        }
        if document.steps.len() > max_steps {
            return Err(WorkflowError::TooManySteps {
                steps: document.steps.len(),
                max_steps,
            });
        }

        let mut indices = HashMap::with_capacity(document.steps.len());
        let mut steps = Vec::with_capacity(document.steps.len());
        for (index, step) in document.steps.into_iter().enumerate() {
            let id = ComponentId::new(step.name)
                .map_err(|_| WorkflowError::InvalidStepName { index })?;
            if step.component.as_os_str().is_empty() {
                return Err(WorkflowError::EmptyComponentPath {
                    step: id.to_string(),
                });
            }
            if indices.insert(id.clone(), index).is_some() {
                return Err(WorkflowError::DuplicateStep {
                    step: id.to_string(),
                });
            }
            let component = if step.component.is_absolute() {
                step.component
            } else {
                base.join(step.component)
            };
            steps.push(WorkflowStep { id, component });
        }

        let (edges, order) = validate_edges(document.edges, &steps, &indices)?;
        let steps: Vec<_> = order
            .into_iter()
            .map(|index| steps[index].clone())
            .collect();
        let mode = match document.mode {
            WorkflowModeDocument::Scalar => WorkflowMode::Scalar,
            WorkflowModeDocument::Stream => WorkflowMode::Stream,
        };
        let input = match (mode, document.input) {
            (WorkflowMode::Scalar, Some(InputDocument::Scalar(input))) => {
                Some(WorkflowInput::Scalar(input))
            }
            (WorkflowMode::Stream, Some(InputDocument::File(input)))
                if !input.as_os_str().is_empty() =>
            {
                Some(WorkflowInput::File(if input.is_absolute() {
                    input
                } else {
                    base.join(input)
                }))
            }
            (WorkflowMode::Stream, None) => None,
            (WorkflowMode::Scalar, _) => {
                return Err(WorkflowError::ScalarInput);
            }
            (WorkflowMode::Stream, _) => {
                return Err(WorkflowError::StreamInput);
            }
        };
        if mode == WorkflowMode::Stream && steps.len() < 2 {
            return Err(WorkflowError::StreamWorkflowSteps);
        }
        if mode == WorkflowMode::Stream
            && edges
                .iter()
                .any(|edge| edge.durability == Durability::Required)
        {
            return Err(WorkflowError::StreamDurability);
        }
        let (wait, wait_after) = parse_wait(document.wait)?;
        let effect = parse_effect(document.effect)?;
        if mode == WorkflowMode::Stream && (wait.is_some() || effect.is_some()) {
            return Err(WorkflowError::StreamControl);
        }
        validate_boundaries(&steps, &edges, wait_after.as_ref(), effect.as_ref())?;
        if mode == WorkflowMode::Scalar && document.result.is_some() {
            return Err(WorkflowError::ScalarStreamResult);
        }
        let stream_result = document
            .result
            .map(|result| {
                if valid_label(&result.high) && valid_label(&result.low) {
                    Ok(StreamResultLabels {
                        high: result.high,
                        low: result.low,
                    })
                } else {
                    Err(WorkflowError::InvalidStreamResult)
                }
            })
            .transpose()?;
        Ok(Self {
            name: document.workflow,
            description: document
                .description
                .filter(|value| !value.trim().is_empty()),
            aliases: document.aliases,
            accepts: document.accepts,
            input,
            mode,
            steps,
            edges,
            wait,
            wait_after,
            effect,
            stream_result,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    pub fn aliases(&self) -> &[String] {
        &self.aliases
    }

    pub fn accepts(&self) -> &[String] {
        &self.accepts
    }

    pub fn matches_name(&self, name: &str) -> bool {
        self.name == name || self.aliases.iter().any(|alias| alias == name)
    }

    pub fn mode(&self) -> WorkflowMode {
        self.mode
    }

    pub fn scalar_input(&self) -> Option<u32> {
        match self.input.as_ref() {
            Some(WorkflowInput::Scalar(input)) => Some(*input),
            _ => None,
        }
    }

    pub fn stream_input(&self) -> Option<&Path> {
        match self.input.as_ref() {
            Some(WorkflowInput::File(path)) => Some(path),
            _ => None,
        }
    }

    pub fn steps(&self) -> &[WorkflowStep] {
        &self.steps
    }

    pub fn edges(&self) -> &[WorkflowEdge] {
        &self.edges
    }

    pub fn stream_result_labels(&self) -> Option<&StreamResultLabels> {
        self.stream_result.as_ref()
    }

    pub fn durability_after_step(&self, index: usize) -> Durability {
        self.steps
            .get(index)
            .and_then(|step| self.edges.iter().find(|edge| edge.from == step.id))
            .map_or(Durability::Ephemeral, |edge| edge.durability)
    }
}

fn read_bounded(path: &Path, max_bytes: usize) -> Result<String, WorkflowError> {
    let file = File::open(path).map_err(|source| WorkflowError::Open {
        path: path.to_path_buf(),
        source,
    })?;
    let limit = u64::try_from(max_bytes).unwrap_or(u64::MAX);
    let mut bytes = Vec::new();
    file.take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|source| WorkflowError::Read {
            path: path.to_path_buf(),
            source,
        })?;
    if bytes.len() > max_bytes {
        return Err(WorkflowError::TooLarge {
            path: path.to_path_buf(),
            max_bytes,
        });
    }
    String::from_utf8(bytes).map_err(|source| WorkflowError::InvalidUtf8 { source })
}
