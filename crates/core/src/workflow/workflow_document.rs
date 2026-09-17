use std::path::PathBuf;

use serde::Deserialize;

use crate::{effect::EffectDocument, wait::WaitDocument};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WorkflowDocument {
    pub(super) workflow: String,
    #[serde(default)]
    pub(super) description: Option<String>,
    #[serde(default)]
    pub(super) aliases: Vec<String>,
    #[serde(default)]
    pub(super) accepts: Vec<String>,
    #[serde(default)]
    pub(super) produces: Vec<String>,
    #[serde(default)]
    pub(super) io: IoDocument,
    #[serde(default)]
    pub(super) resources: Option<ResourcesDocument>,
    #[serde(default)]
    pub(super) mode: WorkflowModeDocument,
    #[serde(default)]
    pub(super) input: Option<InputDocument>,
    pub(super) steps: Vec<StepDocument>,
    pub(super) edges: Vec<EdgeDocument>,
    #[serde(default)]
    pub(super) wait: Option<WaitDocument>,
    #[serde(default)]
    pub(super) effect: Option<EffectDocument>,
    #[serde(default)]
    pub(super) result: Option<StreamResultDocument>,
    #[serde(default)]
    pub(super) output: Option<WorkflowOutputDocument>,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct IoDocument {
    #[serde(default)]
    pub(super) input: IoInputDocument,
    #[serde(default)]
    pub(super) output: IoOutputDocument,
    #[serde(default)]
    pub(super) filename: Option<String>,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum IoInputDocument {
    #[default]
    None,
    File,
    Value,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum IoOutputDocument {
    #[default]
    None,
    Value,
    Artifact,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ResourcesDocument {
    pub(super) fuel: u64,
    pub(super) memory_bytes: u64,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum WorkflowModeDocument {
    #[default]
    Scalar,
    Stream,
    Value,
}

#[derive(Deserialize)]
#[serde(untagged)]
pub(super) enum InputDocument {
    Scalar(u32),
    File(PathBuf),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StepDocument {
    pub(super) name: String,
    pub(super) component: PathBuf,
    #[serde(default)]
    pub(super) hash: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EdgeDocument {
    pub(crate) from: String,
    pub(crate) to: String,
    #[serde(default)]
    pub(crate) durability: DurabilityDocument,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum DurabilityDocument {
    #[default]
    Ephemeral,
    Required,
    Auto,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StreamResultDocument {
    pub(super) high: String,
    pub(super) low: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WorkflowOutputDocument {
    pub(super) filename: String,
    pub(super) content_type: String,
}
