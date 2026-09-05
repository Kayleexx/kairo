use std::{
    fs::File,
    io::Read,
    path::Path,
    time::{Duration, Instant},
};

use kairo_core::{ComponentHash, Config as KairoConfig, Error, Result};
use sha2::{Digest, Sha256};
use wasmtime::{
    Config, Engine, Store, StoreLimits, StoreLimitsBuilder,
    component::{Component, Linker},
};

wasmtime::component::bindgen!({
    world: "probe",
    path: "../../wit",
});

pub struct Runtime {
    engine: Engine,
    config: KairoConfig,
}

pub struct LoadedComponent {
    component: Component,
    hash: ComponentHash,
}

impl LoadedComponent {
    pub fn hash(&self) -> ComponentHash {
        self.hash
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ExecutionResult {
    pub output: u32,
    pub component_hash: ComponentHash,
    pub duration: Duration,
}

struct StoreState {
    limits: StoreLimits,
}

impl Runtime {
    pub fn new(config: KairoConfig) -> Result<Self> {
        let mut engine_config = Config::new();
        engine_config
            .wasm_component_model(true)
            .wasm_component_model_async(config.component_model_async)
            .consume_fuel(true);

        let engine = Engine::new(&engine_config)
            .map_err(|error| Error::new("initialize Wasmtime", format!("{error:#}")))?;
        Ok(Self { engine, config })
    }

    pub fn load_component(&self, path: impl AsRef<Path>) -> Result<LoadedComponent> {
        let path = path.as_ref();
        let bytes = read_bounded(path, self.config.max_component_bytes)?;
        let hash = ComponentHash::sha256(Sha256::digest(&bytes).into());
        let component = Component::new(&self.engine, &bytes).map_err(|error| {
            Error::new(
                "load WebAssembly Component",
                format!("{}: {error:#}", path.display()),
            )
        })?;
        tracing::info!(path = %path.display(), %hash, "component loaded");
        Ok(LoadedComponent { component, hash })
    }

    pub async fn run_component(&self, loaded: &LoadedComponent) -> Result<ExecutionResult> {
        let limits = StoreLimitsBuilder::new()
            .memory_size(self.config.max_memory_bytes)
            .build();
        let mut store = Store::new(&self.engine, StoreState { limits });
        store.limiter(|state| &mut state.limits);
        store
            .set_fuel(self.config.execution_fuel)
            .map_err(|error| Error::new("configure Component fuel", format!("{error:#}")))?;

        let linker = Linker::new(&self.engine);
        let probe = Probe::instantiate_async(&mut store, &loaded.component, &linker)
            .await
            .map_err(|error| {
                Error::new("instantiate WebAssembly Component", format!("{error:#}"))
            })?;

        let started = Instant::now();
        let output = store
            .run_concurrent(async |accessor| probe.call_ping(accessor).await)
            .await
            .map_err(|error| Error::new("run WebAssembly Component", format!("{error:#}")))?
            .map_err(|error| Error::new("invoke probe.ping", format!("{error:#}")))?;
        let duration = started.elapsed();
        tracing::info!(
            hash = %loaded.hash,
            duration_us = duration.as_micros(),
            output,
            "component executed"
        );

        Ok(ExecutionResult {
            output,
            component_hash: loaded.hash,
            duration,
        })
    }
}

fn read_bounded(path: &Path, max_bytes: usize) -> Result<Vec<u8>> {
    let file = File::open(path).map_err(|error| {
        Error::new(
            "open WebAssembly Component",
            format!("{}: {error}", path.display()),
        )
    })?;
    let limit = u64::try_from(max_bytes).unwrap_or(u64::MAX);
    let mut bytes = Vec::new();
    file.take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| {
            Error::new(
                "read WebAssembly Component",
                format!("{}: {error}", path.display()),
            )
        })?;
    if bytes.len() > max_bytes {
        return Err(Error::new(
            "load WebAssembly Component",
            format!("{} exceeds {max_bytes} bytes", path.display()),
        ));
    }
    Ok(bytes)
}
