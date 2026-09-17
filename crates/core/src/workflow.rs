use std::{collections::HashMap, path::Path, str::FromStr};

use crate::effect::{WorkflowEffect, parse_effect};
use crate::wait::{WorkflowWait, parse_wait};
use crate::{
    ComponentHash, ComponentId, Durability, IoInput, IoOutput, StreamResultLabels, WorkflowEdge,
    WorkflowInput, WorkflowIo, WorkflowMode, WorkflowOutput, WorkflowResources, WorkflowStep,
};

pub(crate) use self::workflow_document::{DurabilityDocument, EdgeDocument};
use self::workflow_document::{
    InputDocument, IoInputDocument, IoOutputDocument, WorkflowDocument, WorkflowModeDocument,
};
use self::workflow_graph::{validate_boundaries, validate_edges};
use self::workflow_loading::read_bounded;
use self::workflow_metadata::{valid_label, validate_accepts, validate_aliases, validate_produces};

mod workflow_document;
mod workflow_error;
mod workflow_graph;
mod workflow_loading;
mod workflow_metadata;

pub use workflow_error::WorkflowError;

#[derive(Clone, Debug)]
pub struct Workflow {
    name: String,
    description: Option<String>,
    aliases: Vec<String>,
    accepts: Vec<String>,
    produces: Vec<String>,
    io: WorkflowIo,
    resources: Option<WorkflowResources>,
    input: Option<WorkflowInput>,
    mode: WorkflowMode,
    steps: Vec<WorkflowStep>,
    pub(crate) edges: Vec<WorkflowEdge>,
    pub(crate) wait: Option<WorkflowWait>,
    pub(crate) wait_after: Option<ComponentId>,
    pub(crate) effect: Option<WorkflowEffect>,
    stream_result: Option<StreamResultLabels>,
    output: Option<WorkflowOutput>,
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
        validate_produces(&document.produces)?;
        let io = WorkflowIo {
            input: match document.io.input {
                IoInputDocument::None => IoInput::None,
                IoInputDocument::File => IoInput::File,
                IoInputDocument::Value => IoInput::Value,
            },
            output: match document.io.output {
                IoOutputDocument::None => IoOutput::None,
                IoOutputDocument::Value => IoOutput::Value,
                IoOutputDocument::Artifact => IoOutput::Artifact,
            },
            filename: document
                .io
                .filename
                .map(|filename| {
                    if valid_output_filename(&filename) {
                        Ok(filename)
                    } else {
                        Err(WorkflowError::InvalidIoFilename)
                    }
                })
                .transpose()?,
        };
        let resources = document
            .resources
            .map(|resources| {
                if resources.fuel == 0 || resources.memory_bytes == 0 {
                    return Err(WorkflowError::ZeroResources);
                }
                Ok(WorkflowResources {
                    fuel: resources.fuel,
                    memory_bytes: usize::try_from(resources.memory_bytes)
                        .map_err(|_| WorkflowError::ResourceMemoryOverflow)?,
                })
            })
            .transpose()?;
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
            let pinned_hash = step
                .hash
                .map(|hash| {
                    ComponentHash::from_str(&hash).map_err(|_source| {
                        WorkflowError::InvalidPinnedHash {
                            step: id.to_string(),
                        }
                    })
                })
                .transpose()?;
            steps.push(WorkflowStep {
                id,
                component,
                pinned_hash,
            });
        }

        let (edges, order) = validate_edges(document.edges, &steps, &indices)?;
        let steps: Vec<_> = order
            .into_iter()
            .map(|index| steps[index].clone())
            .collect();
        let mode = match document.mode {
            WorkflowModeDocument::Scalar => WorkflowMode::Scalar,
            WorkflowModeDocument::Stream => WorkflowMode::Stream,
            WorkflowModeDocument::Value => WorkflowMode::Value,
        };
        let input = match (mode, document.input) {
            (WorkflowMode::Scalar, Some(InputDocument::Scalar(input))) => {
                Some(WorkflowInput::Scalar(input))
            }
            (WorkflowMode::Stream | WorkflowMode::Value, Some(InputDocument::File(input)))
                if !input.as_os_str().is_empty() =>
            {
                Some(WorkflowInput::File(if input.is_absolute() {
                    input
                } else {
                    base.join(input)
                }))
            }
            (WorkflowMode::Stream | WorkflowMode::Value, None) => None,
            (WorkflowMode::Scalar, _) => {
                return Err(WorkflowError::ScalarInput);
            }
            (WorkflowMode::Stream, _) => {
                return Err(WorkflowError::StreamInput);
            }
            (WorkflowMode::Value, _) => {
                return Err(WorkflowError::ValueInput);
            }
        };
        if mode == WorkflowMode::Stream && steps.len() < 2 && document.output.is_none() {
            return Err(WorkflowError::StreamWorkflowSteps);
        }
        if mode == WorkflowMode::Stream
            && edges.iter().any(|edge| edge.durability == Durability::Auto)
        {
            return Err(WorkflowError::StreamDurability);
        }
        let (wait, wait_after) = parse_wait(document.wait)?;
        let effect = parse_effect(document.effect)?;
        if mode == WorkflowMode::Stream && (wait.is_some() || effect.is_some()) {
            return Err(WorkflowError::StreamControl);
        }
        validate_boundaries(&steps, &edges, wait_after.as_ref(), effect.as_ref())?;
        if matches!(mode, WorkflowMode::Scalar | WorkflowMode::Value) && document.result.is_some() {
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
        if matches!(mode, WorkflowMode::Scalar | WorkflowMode::Value) && document.output.is_some() {
            return Err(WorkflowError::ScalarOutput);
        }
        let output = document
            .output
            .map(|output| {
                if !valid_output_filename(&output.filename) {
                    return Err(WorkflowError::InvalidOutputFilename);
                }
                if !valid_output_content_type(&output.content_type) {
                    return Err(WorkflowError::InvalidOutputContentType);
                }
                Ok(WorkflowOutput {
                    filename: output.filename,
                    content_type: output.content_type,
                })
            })
            .transpose()?;
        Ok(Self {
            name: document.workflow,
            description: document
                .description
                .filter(|value| !value.trim().is_empty()),
            aliases: document.aliases,
            accepts: document.accepts,
            produces: document.produces,
            io,
            resources,
            input,
            mode,
            steps,
            edges,
            wait,
            wait_after,
            effect,
            stream_result,
            output,
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

    pub fn produces(&self) -> &[String] {
        &self.produces
    }

    pub fn io(&self) -> &WorkflowIo {
        &self.io
    }

    pub fn resources(&self) -> Option<WorkflowResources> {
        self.resources
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

    pub fn output(&self) -> Option<&WorkflowOutput> {
        self.output.as_ref()
    }

    pub fn durability_after_step(&self, index: usize) -> Durability {
        self.steps
            .get(index)
            .and_then(|step| self.edges.iter().find(|edge| edge.from == step.id))
            .map_or(Durability::Ephemeral, |edge| edge.durability)
    }
}

fn valid_output_filename(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value != "."
        && value != ".."
        && !value.contains("..")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_graphic() && byte != b'/' && byte != b'\\')
}

fn valid_output_content_type(value: &str) -> bool {
    !value.is_empty() && value.len() <= 128 && value.bytes().all(|byte| byte.is_ascii_graphic())
}
