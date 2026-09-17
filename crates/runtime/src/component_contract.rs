use std::path::Path;

use kairo_core::{ComponentHash, Config, WorkflowMode};
use wasmtime::component::InstancePre;

use super::{Runtime, StoreState};

mod stage {
    wasmtime::component::bindgen!({ world: "stage", path: "../../wit" });
}
mod value_stage {
    wasmtime::component::bindgen!({ world: "value-stage", path: "../../wit" });
}
mod transform {
    wasmtime::component::bindgen!({ world: "transform", path: "../../wit" });
}
mod consume {
    wasmtime::component::bindgen!({ world: "consume", path: "../../wit" });
}
mod consume_metrics {
    wasmtime::component::bindgen!({ world: "consume-metrics", path: "../../wit" });
}
mod output {
    wasmtime::component::bindgen!({ world: "output", path: "../../wit" });
}

/// every role a Component can implement against a Kairo-supported world today -- nothing
/// arbitrary-WIT, nothing speculative. Mirrors the worlds bound in `workflow.rs` (`stage`),
/// `workflow/value.rs` (`value-stage`), and `stream.rs`
/// (`transform`/`consume`/`consume-metrics`/`output`).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum ComponentRole {
    ValueStage,
    ScalarStage,
    StreamTransform,
    StreamConsume,
    StreamConsumeMetrics,
    StreamOutput,
}

const ROLES: [ComponentRole; 6] = [
    ComponentRole::ValueStage,
    ComponentRole::ScalarStage,
    ComponentRole::StreamTransform,
    ComponentRole::StreamConsume,
    ComponentRole::StreamConsumeMetrics,
    ComponentRole::StreamOutput,
];

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
            Self::ValueStage => "value \u{2192} value",
            Self::ScalarStage => "number \u{2192} number",
            Self::StreamTransform => "byte stream \u{2192} byte stream",
            Self::StreamConsume => "byte stream \u{2192} number",
            Self::StreamConsumeMetrics => "byte stream \u{2192} value",
            Self::StreamOutput => "byte stream \u{2192} artifact",
        }
    }

    fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::StreamConsume | Self::StreamConsumeMetrics | Self::StreamOutput
        )
    }

    /// whether a chain may legally end right after a step with this role -- false only for
    /// `StreamTransform`, which must always be followed by a terminal.
    pub fn allows_finish(self) -> bool {
        self != Self::StreamTransform
    }

    /// a stream chain is zero-or-more `StreamTransform` steps followed by exactly one terminal
    /// (`StreamConsume`/`StreamConsumeMetrics`/`StreamOutput`); `ValueStage`/`ScalarStage`
    /// workflows are a uniform chain of that one role. `previous == None` means "first step",
    /// always legal.
    pub fn can_follow(self, previous: Option<ComponentRole>) -> bool {
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
            Some(role) if role.is_terminal() => false,
            Some(_) => false,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ComponentContract {
    pub role: ComponentRole,
    pub hash: ComponentHash,
}

impl ComponentContract {
    pub fn mode(&self) -> WorkflowMode {
        self.role.mode()
    }

    pub fn shape(&self) -> &'static str {
        self.role.shape()
    }

    pub fn can_follow(&self, previous: Option<ComponentRole>) -> bool {
        self.role.can_follow(previous)
    }
}

/// classifies a single Component against every world the runtime actually supports, entirely
/// independent of any workflow document -- direct `instantiate_pre` + typed `*Pre::new` checks,
/// the same type-checked instantiation `prepare_workflow`/`prepare_stream_workflow` do for a real
/// run, just without assembling a whole workflow around it first. Read-only: never runs the
/// Component, never touches full `validate_workflow`.
pub fn detect_contract(component: &Path, config: Config) -> Option<ComponentContract> {
    let runtime = Runtime::new(config).ok()?;
    let loaded = runtime.load_component(component).ok()?;
    let linker = runtime.component_linker().ok()?;
    for role in ROLES {
        let Ok(pre) = linker.instantiate_pre(&loaded.component) else {
            continue;
        };
        if matches_role(role, pre) {
            return Some(ComponentContract {
                role,
                hash: loaded.hash,
            });
        }
    }
    None
}

fn matches_role(role: ComponentRole, pre: InstancePre<StoreState>) -> bool {
    match role {
        ComponentRole::ValueStage => value_stage::ValueStagePre::new(pre).is_ok(),
        ComponentRole::ScalarStage => stage::StagePre::new(pre).is_ok(),
        ComponentRole::StreamTransform => transform::TransformPre::new(pre).is_ok(),
        ComponentRole::StreamConsume => consume::ConsumePre::new(pre).is_ok(),
        ComponentRole::StreamConsumeMetrics => consume_metrics::ConsumeMetricsPre::new(pre).is_ok(),
        ComponentRole::StreamOutput => output::OutputPre::new(pre).is_ok(),
    }
}
