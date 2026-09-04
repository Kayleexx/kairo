use std::path::Path;

use kairo_core::{Config as KairoConfig, Error, Result};
use wasmtime::{Config, Engine, component::Component};

pub struct Runtime {
    engine: Engine,
}

impl Runtime {
    pub fn new(config: KairoConfig) -> Result<Self> {
        let mut engine_config = Config::new();
        engine_config
            .wasm_component_model(true)
            .wasm_component_model_async(config.component_model_async);

        let engine = Engine::new(&engine_config)
            .map_err(|error| Error::new("initialize Wasmtime", error.to_string()))?;
        Ok(Self { engine })
    }

    pub fn load_component(&self, path: impl AsRef<Path>) -> Result<Component> {
        let path = path.as_ref();
        let component = Component::from_file(&self.engine, path).map_err(|error| {
            Error::new(
                "load WebAssembly Component",
                format!("{}: {error}", path.display()),
            )
        })?;
        tracing::info!(path = %path.display(), "component loaded");
        Ok(component)
    }
}
