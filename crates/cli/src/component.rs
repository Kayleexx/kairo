use std::path::Path;

use kairo_core::Config;
use kairo_tui::scaffold::{self, ScaffoldError};

use crate::{args::ComponentCommand, print_valid, validation};

pub(crate) type ComponentError = ScaffoldError;

pub(crate) fn dispatch(command: ComponentCommand, config: Config) -> crate::Result<()> {
    match command {
        ComponentCommand::New { name } => {
            let directory = new(&name)?;
            print_valid(format!(
                "component · {}\nnext: edit src/lib.rs (and Cargo.toml's `description`, so it \
                 shows up in `kairo new`), then run `kairo component build {}`",
                directory.display(),
                directory.display()
            ));
        }
        ComponentCommand::Build { path } => {
            let component_path = build(&path)?;
            print_valid(format!("component · {}", component_path.display()));
        }
        ComponentCommand::Check { path } => validation::component(&path, config)?,
    }
    Ok(())
}

pub(crate) fn new(name: &str) -> Result<std::path::PathBuf, ComponentError> {
    scaffold::new(name)
}

pub(crate) fn build(path: &Path) -> Result<std::path::PathBuf, ComponentError> {
    scaffold::build(path)
}

pub(crate) fn valid_name(name: &str) -> bool {
    scaffold::valid_name(name)
}
