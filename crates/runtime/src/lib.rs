use std::{
    fs::File,
    io::Read,
    path::Path,
    time::{Duration, Instant},
};

use kairo_core::{ComponentHash, Config as KairoConfig};
use sha2::{Digest, Sha256};
use wasmtime::{
    Config, Engine, ResourceLimiter, Store, StoreLimits, StoreLimitsBuilder, Trap,
    component::{Component, Linker},
};

wasmtime::component::bindgen!({
    world: "probe",
    path: "../../wit",
});

mod cell;
mod error;
mod identity;
mod inspection;
mod inspection_events;
mod journal;
mod journal_event;
mod local_state;
mod receipt;
mod stream;
mod stream_input;
mod stream_run;
mod workflow;
mod workflow_wait;

pub use error::{Result, RuntimeError};
pub use inspection::{CellInspection, CellStatus, ComponentInspection, inspect_cell};
pub use inspection_events::{CellEvent, inspect_events};
pub use journal::JournalError;
pub use local_state::{LocalCell, LocalStateError, discover_cells, discover_cells_in};
pub use receipt::{EffectReceipt, inspect_receipts};
pub use stream::{StreamMetrics, StreamResult};
pub use stream_run::{
    StreamRun, StreamRunError, StreamRunInspection, StreamRunStatus, inspect_stream_run,
};
pub use workflow::{CellRunResult, WorkflowResult};
pub use workflow_wait::{
    DurableWait, WorkflowWaitError, WorkflowWaitState, complete_workflow_wait,
    inspect_workflow_wait, record_workflow_wait,
};

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

#[derive(Clone, Copy)]
enum CallKind {
    Runtime,
    Component,
    Workflow,
}

struct StoreState {
    limits: StoreLimits,
    memory_limit_reached: bool,
    stream_metrics: Option<StreamMetrics>,
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

impl Runtime {
    pub fn new(config: KairoConfig) -> Result<Self> {
        if config.stream_chunk_bytes == 0 {
            return Err(RuntimeError::InvalidStreamChunkSize);
        }
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

    pub async fn run_component(
        &self,
        loaded: &LoadedComponent,
        input: u32,
    ) -> Result<ExecutionResult> {
        let mut store = self.new_store()?;

        let linker = self.component_linker()?;
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
            .run_concurrent(async |accessor| probe.call_compute(accessor, input).await)
            .await
        {
            Ok(Ok(output)) => output,
            Ok(Err(source)) => {
                return Err(self.execution_error(source, &store, CallKind::Component));
            }
            Err(source) => {
                return Err(self.execution_error(source, &store, CallKind::Runtime));
            }
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

    fn component_linker(&self) -> Result<Linker<StoreState>> {
        let mut linker = Linker::new(&self.engine);
        if self.config.allow_console {
            linker
                .root()
                .func_wrap(
                    "console",
                    |_store, (value,): (u32,)| -> wasmtime::Result<()> {
                        eprintln!("guest: {value}");
                        Ok(())
                    },
                )
                .map_err(|source| RuntimeError::ConfigureCapability {
                    capability: "console",
                    source,
                })?;
        }
        Ok(linker)
    }

    fn new_store(&self) -> Result<Store<StoreState>> {
        let limits = StoreLimitsBuilder::new()
            .memory_size(self.config.max_memory_bytes)
            .build();
        let mut store = Store::new(
            &self.engine,
            StoreState {
                limits,
                memory_limit_reached: false,
                stream_metrics: None,
            },
        );
        store.limiter(|state| state);
        store
            .set_fuel(self.config.execution_fuel)
            .map_err(|source| RuntimeError::ConfigureFuel { source })?;
        Ok(store)
    }

    fn instantiation_error(
        &self,
        source: wasmtime::Error,
        store: &Store<StoreState>,
    ) -> RuntimeError {
        if store.data().memory_limit_reached {
            RuntimeError::MemoryLimitExceeded {
                max_memory_bytes: self.config.max_memory_bytes,
                source,
            }
        } else {
            RuntimeError::Instantiate { source }
        }
    }

    fn execution_error(
        &self,
        source: wasmtime::Error,
        store: &Store<StoreState>,
        call: CallKind,
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
        } else {
            match call {
                CallKind::Runtime => RuntimeError::Execute { source },
                CallKind::Component => RuntimeError::InvokeComponent { source },
                CallKind::Workflow => RuntimeError::InvokeWorkflow { source },
            }
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
