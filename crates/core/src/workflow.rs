use std::{
    collections::{HashMap, HashSet, VecDeque},
    fs::File,
    io::{self, Read},
    path::{Path, PathBuf},
};

use serde::Deserialize;
use thiserror::Error;

use crate::effect::{EffectDocument, WorkflowEffect, parse_effect};
use crate::wait::{WaitDocument, WorkflowWait, parse_wait};
use crate::{ComponentId, Durability, WorkflowEdge, WorkflowInput, WorkflowMode, WorkflowStep};

#[derive(Clone, Debug)]
pub struct Workflow {
    name: String,
    input: WorkflowInput,
    mode: WorkflowMode,
    steps: Vec<WorkflowStep>,
    pub(crate) edges: Vec<WorkflowEdge>,
    pub(crate) wait: Option<WorkflowWait>,
    pub(crate) effect: Option<WorkflowEffect>,
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
    #[error("stream workflows require exactly two steps")]
    StreamWorkflowSteps,
    #[error("stream workflows do not support `durability: required`")]
    StreamDurability,
    #[error("workflow wait must specify exactly one of `timer_ms` or `signal`")]
    InvalidWait,
    #[error("workflow effect operation must be 1–64 letters, digits, `-`, or `_`")]
    InvalidEffect,
    #[error("stream workflows do not support waits or effects")]
    StreamControl,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkflowDocument {
    workflow: String,
    #[serde(default)]
    mode: WorkflowModeDocument,
    input: InputDocument,
    steps: Vec<StepDocument>,
    edges: Vec<EdgeDocument>,
    #[serde(default)]
    wait: Option<WaitDocument>,
    #[serde(default)]
    effect: Option<EffectDocument>,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "lowercase")]
enum WorkflowModeDocument {
    #[default]
    Scalar,
    Stream,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum InputDocument {
    Scalar(u32),
    File(PathBuf),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StepDocument {
    name: String,
    component: PathBuf,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EdgeDocument {
    from: String,
    to: String,
    #[serde(default)]
    durability: DurabilityDocument,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "lowercase")]
enum DurabilityDocument {
    #[default]
    Ephemeral,
    Required,
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
            (WorkflowMode::Scalar, InputDocument::Scalar(input)) => WorkflowInput::Scalar(input),
            (WorkflowMode::Stream, InputDocument::File(input)) if !input.as_os_str().is_empty() => {
                WorkflowInput::File(if input.is_absolute() {
                    input
                } else {
                    base.join(input)
                })
            }
            (WorkflowMode::Scalar, InputDocument::File(_)) => {
                return Err(WorkflowError::ScalarInput);
            }
            (WorkflowMode::Stream, InputDocument::Scalar(_)) => {
                return Err(WorkflowError::StreamInput);
            }
            (WorkflowMode::Stream, InputDocument::File(_)) => {
                return Err(WorkflowError::StreamInput);
            }
        };
        if mode == WorkflowMode::Stream && steps.len() != 2 {
            return Err(WorkflowError::StreamWorkflowSteps);
        }
        if mode == WorkflowMode::Stream
            && edges
                .iter()
                .any(|edge| edge.durability == Durability::Required)
        {
            return Err(WorkflowError::StreamDurability);
        }
        let wait = parse_wait(document.wait)?;
        let effect = parse_effect(document.effect)?;
        if mode == WorkflowMode::Stream && (wait.is_some() || effect.is_some()) {
            return Err(WorkflowError::StreamControl);
        }
        Ok(Self {
            name: document.workflow,
            input,
            mode,
            steps,
            edges,
            wait,
            effect,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn mode(&self) -> WorkflowMode {
        self.mode
    }

    pub fn scalar_input(&self) -> Option<u32> {
        match self.input {
            WorkflowInput::Scalar(input) => Some(input),
            WorkflowInput::File(_) => None,
        }
    }

    pub fn stream_input(&self) -> Option<&Path> {
        match &self.input {
            WorkflowInput::Scalar(_) => None,
            WorkflowInput::File(path) => Some(path),
        }
    }

    pub fn steps(&self) -> &[WorkflowStep] {
        &self.steps
    }

    pub fn edges(&self) -> &[WorkflowEdge] {
        &self.edges
    }

    pub fn durability_after_step(&self, index: usize) -> Durability {
        self.steps
            .get(index)
            .and_then(|step| self.edges.iter().find(|edge| edge.from == step.id))
            .map_or(Durability::Ephemeral, |edge| edge.durability)
    }
}

fn validate_edges(
    documents: Vec<EdgeDocument>,
    steps: &[WorkflowStep],
    indices: &HashMap<ComponentId, usize>,
) -> Result<(Vec<WorkflowEdge>, Vec<usize>), WorkflowError> {
    let mut adjacency = vec![Vec::new(); steps.len()];
    let mut indegree = vec![0usize; steps.len()];
    let mut seen = HashSet::with_capacity(documents.len());
    let mut edges = Vec::with_capacity(documents.len());

    for (index, edge) in documents.into_iter().enumerate() {
        let from = ComponentId::new(edge.from).map_err(|_| WorkflowError::EmptyEdgeStep {
            index,
            endpoint: "from",
        })?;
        let to = ComponentId::new(edge.to).map_err(|_| WorkflowError::EmptyEdgeStep {
            index,
            endpoint: "to",
        })?;
        let from_index = *indices
            .get(&from)
            .ok_or_else(|| WorkflowError::UnknownStep {
                step: from.to_string(),
            })?;
        let to_index = *indices.get(&to).ok_or_else(|| WorkflowError::UnknownStep {
            step: to.to_string(),
        })?;
        if !seen.insert((from_index, to_index)) {
            return Err(WorkflowError::DuplicateEdge {
                from: from.to_string(),
                to: to.to_string(),
            });
        }
        adjacency[from_index].push(to_index);
        indegree[to_index] += 1;
        edges.push(WorkflowEdge {
            from,
            to,
            durability: match edge.durability {
                DurabilityDocument::Ephemeral => Durability::Ephemeral,
                DurabilityDocument::Required => Durability::Required,
            },
        });
    }

    for (index, outputs) in adjacency.iter().enumerate() {
        if outputs.len() > 1 {
            return Err(WorkflowError::MultipleOutputs {
                step: steps[index].id.to_string(),
            });
        }
        if indegree[index] > 1 {
            return Err(WorkflowError::MultipleInputs {
                step: steps[index].id.to_string(),
            });
        }
    }

    let mut ready: VecDeque<_> = indegree
        .iter()
        .enumerate()
        .filter_map(|(index, degree)| (*degree == 0).then_some(index))
        .collect();
    let mut order = Vec::with_capacity(steps.len());
    while let Some(index) = ready.pop_front() {
        order.push(index);
        for &next in &adjacency[index] {
            indegree[next] -= 1;
            if indegree[next] == 0 {
                ready.push_back(next);
            }
        }
    }
    if order.len() != steps.len() {
        return Err(WorkflowError::Cycle);
    }
    if edges.len().saturating_add(1) != steps.len() {
        return Err(WorkflowError::Disconnected);
    }
    Ok((edges, order))
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
