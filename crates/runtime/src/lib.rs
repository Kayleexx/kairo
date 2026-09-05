use std::{
    fs::File,
    io::{self, Read},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use kairo_core::{ComponentHash, Config as KairoConfig};
use sha2::{Digest, Sha256};
use thiserror::Error;
use wasmtime::{
    Config, Engine, ResourceLimiter, Store, StoreLimits, StoreLimitsBuilder, Trap,
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
    memory_limit_reached: bool,
}

impl ResourceLimiter for StoreState {
    fn memory_growing(
        &mut self,
        current: usize,
        desired: usize,
        maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        let allowed = self.limits.memory_growing(current, desired, maximum)?;
        self.memory_limit_reached |= !allowed;
        Ok(allowed)
    }

    fn table_growing(
        &mut self,
        current: usize,
        desired: usize,
        maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        self.limits.table_growing(current, desired, maximum)
    }

    fn instances(&self) -> usize {
        self.limits.instances()
    }

    fn tables(&self) -> usize {
        self.limits.tables()
    }

    fn memories(&self) -> usize {
        self.limits.memories()
    }
}

#[derive(Debug, Error)]
pub enum RuntimeError {
    #[error("failed to initialize Wasmtime")]
    Engine {
        #[source]
        source: wasmtime::Error,
    },
    #[error("failed to open component `{path}`")]
    OpenComponent {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to read component `{path}`")]
    ReadComponent {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("component `{path}` exceeds the {max_bytes}-byte size limit")]
    ComponentTooLarge { path: PathBuf, max_bytes: usize },
    #[error("component `{path}` is invalid")]
    InvalidComponent {
        path: PathBuf,
        #[source]
        source: wasmtime::Error,
    },
    #[error("failed to configure component fuel")]
    ConfigureFuel {
        #[source]
        source: wasmtime::Error,
    },
    #[error("failed to instantiate component")]
    Instantiate {
        #[source]
        source: wasmtime::Error,
    },
    #[error("component exhausted its {fuel}-fuel limit")]
    FuelExhausted {
        fuel: u64,
        #[source]
        source: wasmtime::Error,
    },
    #[error("component exceeded its {max_memory_bytes}-byte memory limit")]
    MemoryLimitExceeded {
        max_memory_bytes: usize,
        #[source]
        source: wasmtime::Error,
    },
    #[error("component execution failed")]
    Execute {
        #[source]
        source: wasmtime::Error,
    },
    #[error("failed to invoke `probe.ping`")]
    InvokeProbe {
        #[source]
        source: wasmtime::Error,
    },
}

pub type Result<T> = std::result::Result<T, RuntimeError>;

impl Runtime {
    pub fn new(config: KairoConfig) -> Result<Self> {
        let mut engine_config = Config::new();
        engine_config
            .wasm_component_model(true)
            .wasm_component_model_async(config.component_model_async)
            .consume_fuel(true);

        let engine =
            Engine::new(&engine_config).map_err(|source| RuntimeError::Engine { source })?;
        Ok(Self { engine, config })
    }

    pub fn load_component(&self, path: impl AsRef<Path>) -> Result<LoadedComponent> {
        let path = path.as_ref();
        let bytes = read_bounded(path, self.config.max_component_bytes)?;
        let hash = ComponentHash::sha256(Sha256::digest(&bytes).into());
        let component = Component::new(&self.engine, &bytes).map_err(|source| {
            RuntimeError::InvalidComponent {
                path: path.to_path_buf(),
                source,
            }
        })?;
        tracing::info!(path = %path.display(), %hash, "component loaded");
        Ok(LoadedComponent { component, hash })
    }

    pub async fn run_component(&self, loaded: &LoadedComponent) -> Result<ExecutionResult> {
        let limits = StoreLimitsBuilder::new()
            .memory_size(self.config.max_memory_bytes)
            .build();
        let mut store = Store::new(
            &self.engine,
            StoreState {
                limits,
                memory_limit_reached: false,
            },
        );
        store.limiter(|state| state);
        store
            .set_fuel(self.config.execution_fuel)
            .map_err(|source| RuntimeError::ConfigureFuel { source })?;

        let linker = Linker::new(&self.engine);
        let probe = match Probe::instantiate_async(&mut store, &loaded.component, &linker).await {
            Ok(probe) => probe,
            Err(source) if store.data().memory_limit_reached => {
                return Err(RuntimeError::MemoryLimitExceeded {
                    max_memory_bytes: self.config.max_memory_bytes,
                    source,
                });
            }
            Err(source) => return Err(RuntimeError::Instantiate { source }),
        };

        let started = Instant::now();
        let output = match store
            .run_concurrent(async |accessor| probe.call_ping(accessor).await)
            .await
        {
            Ok(Ok(output)) => output,
            Ok(Err(source)) => return Err(self.execution_error(source, &store, true)),
            Err(source) => return Err(self.execution_error(source, &store, false)),
        };
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

    fn execution_error(
        &self,
        source: wasmtime::Error,
        store: &Store<StoreState>,
        invoked_probe: bool,
    ) -> RuntimeError {
        if store.data().memory_limit_reached {
            RuntimeError::MemoryLimitExceeded {
                max_memory_bytes: self.config.max_memory_bytes,
                source,
            }
        } else if source.downcast_ref::<Trap>() == Some(&Trap::OutOfFuel) {
            RuntimeError::FuelExhausted {
                fuel: self.config.execution_fuel,
                source,
            }
        } else if invoked_probe {
            RuntimeError::InvokeProbe { source }
        } else {
            RuntimeError::Execute { source }
        }
    }
}

fn read_bounded(path: &Path, max_bytes: usize) -> Result<Vec<u8>> {
    let file = File::open(path).map_err(|source| RuntimeError::OpenComponent {
        path: path.to_path_buf(),
        source,
    })?;
    let limit = u64::try_from(max_bytes).unwrap_or(u64::MAX);
    let mut bytes = Vec::new();
    file.take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|source| RuntimeError::ReadComponent {
            path: path.to_path_buf(),
            source,
        })?;
    if bytes.len() > max_bytes {
        return Err(RuntimeError::ComponentTooLarge {
            path: path.to_path_buf(),
            max_bytes,
        });
    }
    Ok(bytes)
}
