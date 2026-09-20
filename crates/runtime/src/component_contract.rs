use std::path::Path;

use kairo_core::{ComponentHash, ComponentRole, Config, WorkflowMode, catalog::ComponentEntry};
use wasmtime::component::InstancePre;

use super::{Runtime, RuntimeError, StoreState, read_bounded};

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

const ROLES: [ComponentRole; 6] = [
    ComponentRole::ValueStage,
    ComponentRole::ScalarStage,
    ComponentRole::StreamTransform,
    ComponentRole::StreamConsume,
    ComponentRole::StreamConsumeMetrics,
    ComponentRole::StreamOutput,
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ComponentContract {
    pub role: ComponentRole,
    pub hash: ComponentHash,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ComponentDescriptor {
    pub contract: ComponentContract,
    pub package: Option<String>,
    pub version: Option<String>,
    pub world: String,
    pub imports: Vec<String>,
    pub exports: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct CatalogComponent {
    pub entry: ComponentEntry,
    pub contract: ComponentContract,
}

pub fn catalog_components(config: Config) -> Vec<CatalogComponent> {
    kairo_core::catalog::list(&[Path::new("components"), Path::new("components/reference")])
        .into_iter()
        .filter_map(|entry| {
            let contract = detect_contract(&entry.path, config)?;
            Some(CatalogComponent { entry, contract })
        })
        .collect()
}

pub fn compatible_components(
    components: &[CatalogComponent],
    previous: Option<ComponentRole>,
) -> Vec<&CatalogComponent> {
    components
        .iter()
        .filter(|component| component.contract.can_follow(previous))
        .collect()
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
    detect_loaded(&runtime, &loaded)
}

pub fn inspect_contract(
    component: &Path,
    config: Config,
) -> Result<ComponentDescriptor, RuntimeError> {
    let runtime = Runtime::new(config)?;
    let loaded = runtime.load_component(component)?;
    let contract =
        detect_loaded(&runtime, &loaded).ok_or_else(|| RuntimeError::DecodeComponentContract {
            path: component.to_path_buf(),
            message: "the Component does not export a supported Kairo world".to_owned(),
        })?;
    let bytes = read_bounded(component, config.max_component_bytes)?;
    let binary =
        wat::parse_bytes(&bytes).map_err(|error| RuntimeError::DecodeComponentContract {
            path: component.to_path_buf(),
            message: error.to_string(),
        })?;
    let decoded =
        wit_component::decode(&binary).map_err(|error| RuntimeError::DecodeComponentContract {
            path: component.to_path_buf(),
            message: error.to_string(),
        })?;
    let wit_component::DecodedWasm::Component(resolve, world_id) = decoded else {
        return Err(RuntimeError::DecodeComponentContract {
            path: component.to_path_buf(),
            message: "expected a WebAssembly Component, found an encoded WIT package".to_owned(),
        });
    };
    let world = &resolve.worlds[world_id];
    let package = world
        .package
        .map(|id| resolve.packages[id].name.to_string());
    let version = world.package.and_then(|id| {
        resolve.packages[id]
            .name
            .version
            .as_ref()
            .map(ToString::to_string)
    });
    let mut imports = world
        .imports
        .keys()
        .map(|key| resolve.name_world_key(key))
        .collect::<Vec<_>>();
    let mut exports = world
        .exports
        .keys()
        .map(|key| resolve.name_world_key(key))
        .collect::<Vec<_>>();
    imports.sort();
    exports.sort();
    Ok(ComponentDescriptor {
        contract,
        package,
        version,
        world: world.name.clone(),
        imports,
        exports,
    })
}

fn detect_loaded(runtime: &Runtime, loaded: &super::LoadedComponent) -> Option<ComponentContract> {
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
