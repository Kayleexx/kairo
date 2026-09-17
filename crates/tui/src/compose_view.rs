use std::{
    collections::HashSet,
    fs::OpenOptions,
    io::Write as _,
    path::{Path, PathBuf},
};

use crossterm::event::KeyCode;
use kairo_core::{ComponentHash, Config, Durability, Workflow, WorkflowMode};
use kairo_runtime::{ComponentRole, Runtime};

use crate::{App, Screen, compose, scaffold};

pub(crate) enum ComposeStage {
    Name,
    Step,
    UnknownMenu { name: String },
    Import,
    AddAnother,
    OutputFilename,
    OutputContentType,
}

impl App {
    pub(crate) fn enter_compose(&mut self) {
        self.screen = Screen::Compose;
        self.compose_stage = ComposeStage::Name;
        self.compose_name.clear();
        self.compose_input.clear();
        self.compose_paths.clear();
        self.compose_hashes.clear();
        self.compose_step_names.clear();
        self.compose_role = None;
        self.compose_candidates = Vec::new();
        self.compose_selected = 0;
        self.compose_output_filename.clear();
        self.compose_error = None;
        self.dirty = true;
    }

    fn leave_compose(&mut self, notice: String) {
        self.screen = Screen::Launch;
        self.catalog = crate::launch::catalog();
        self.notice = Some(notice);
        self.dirty = true;
    }

    fn refresh_candidates(&mut self) {
        let catalog = compose::catalog(Config::default());
        self.compose_candidates = compose::compatible(&catalog, self.compose_role)
            .into_iter()
            .cloned()
            .collect();
        self.compose_selected = 0;
    }

    pub(crate) fn filtered(&self) -> Vec<&compose::CatalogComponent> {
        let needle = self.compose_input.trim().to_ascii_lowercase();
        self.compose_candidates
            .iter()
            .filter(|component| {
                needle.is_empty()
                    || component.entry.name.to_ascii_lowercase().contains(&needle)
                    || component
                        .entry
                        .description
                        .as_deref()
                        .is_some_and(|description| {
                            description.to_ascii_lowercase().contains(&needle)
                        })
            })
            .collect()
    }

    fn push_step(
        &mut self,
        path: PathBuf,
        hash: Option<ComponentHash>,
        name: String,
        role: ComponentRole,
    ) {
        self.compose_step_names.push(name);
        self.compose_paths.push(path);
        self.compose_hashes.push(hash);
        self.compose_role = Some(role);
        self.compose_input.clear();
        self.compose_error = None;
        if compose::finishable(role, self.compose_paths.len()) {
            self.compose_stage = ComposeStage::AddAnother;
        } else {
            self.refresh_candidates();
            self.compose_stage = ComposeStage::Step;
        }
        self.dirty = true;
    }

    fn unique_step_name(&self, base: &str) -> String {
        let used: HashSet<&str> = self.compose_step_names.iter().map(String::as_str).collect();
        if !used.contains(base) {
            return base.to_owned();
        }
        let mut suffix = 2;
        loop {
            let candidate = format!("{base}-{suffix}");
            if !used.contains(candidate.as_str()) {
                return candidate;
            }
            suffix += 1;
        }
    }

    fn finalize(&mut self) {
        let role = self.compose_role.unwrap_or(ComponentRole::ValueStage);
        let mode = role.mode();
        let output = (role == ComponentRole::StreamOutput).then(|| {
            (
                self.compose_output_filename.clone(),
                "application/octet-stream".to_owned(),
            )
        });
        let durabilities = vec![Durability::Auto; self.compose_paths.len().saturating_sub(1)];
        let source = compose::render(
            &self.compose_name,
            mode,
            0,
            &self.compose_paths,
            &self.compose_hashes,
            &self.compose_step_names,
            &durabilities,
            output,
            None,
            None,
        );
        if let Err(error) = self.write_workflow(&source) {
            self.compose_error = Some(error);
            self.compose_stage = ComposeStage::Step;
            self.dirty = true;
            return;
        }
        let hint = match mode {
            WorkflowMode::Value => format!("kairo run {} --value <input>", self.compose_name),
            WorkflowMode::Stream => format!("kairo run {} <input-file>", self.compose_name),
            WorkflowMode::Scalar => format!("kairo run {}", self.compose_name),
        };
        self.leave_compose(format!("✓ {} created · next · {hint}", self.compose_name));
    }

    fn write_workflow(&self, source: &str) -> Result<(), String> {
        let config = Config::default();
        let workflow = Workflow::parse(source, Path::new("."), config.max_workflow_steps)
            .map_err(|error| error.to_string())?;
        let runtime = Runtime::new(config).map_err(|error| error.to_string())?;
        runtime
            .validate_workflow(&workflow)
            .map_err(|error| error.to_string())?;
        let path = PathBuf::from(format!("{}.yaml", self.compose_name));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        file.write_all(source.as_bytes())
            .map_err(|error| format!("{}: {error}", path.display()))?;
        Ok(())
    }

    pub(crate) fn compose_key(&mut self, code: KeyCode) {
        match &self.compose_stage {
            ComposeStage::Name => self.compose_key_name(code),
            ComposeStage::Step => self.compose_key_step(code),
            ComposeStage::UnknownMenu { .. } => self.compose_key_unknown_menu(code),
            ComposeStage::Import => self.compose_key_import(code),
            ComposeStage::AddAnother => self.compose_key_add_another(code),
            ComposeStage::OutputFilename => self.compose_key_output_filename(code),
            ComposeStage::OutputContentType => self.compose_key_output_content_type(code),
        }
        self.dirty = true;
    }

    fn compose_key_name(&mut self, code: KeyCode) {
        match code {
            KeyCode::Esc => self.leave_compose("workflow creation canceled".to_owned()),
            KeyCode::Backspace => {
                self.compose_input.pop();
            }
            KeyCode::Char(character) => self.compose_input.push(character),
            KeyCode::Enter => {
                let name = self.compose_input.trim().to_owned();
                if !scaffold::valid_name(&name) {
                    self.compose_error =
                        Some("use 1-64 lowercase letters, digits, `-`, or `_`".to_owned());
                    return;
                }
                self.compose_name = name;
                self.compose_input.clear();
                self.compose_error = None;
                self.refresh_candidates();
                self.compose_stage = ComposeStage::Step;
            }
            _ => {}
        }
    }

    fn compose_key_step(&mut self, code: KeyCode) {
        match code {
            KeyCode::Esc => self.leave_compose("workflow creation canceled".to_owned()),
            KeyCode::Up => {
                let total = self.filtered().len();
                if total > 0 {
                    self.compose_selected =
                        self.compose_selected.checked_sub(1).unwrap_or(total - 1);
                }
            }
            KeyCode::Down => {
                let total = self.filtered().len();
                if total > 0 {
                    self.compose_selected = (self.compose_selected + 1) % total;
                }
            }
            KeyCode::Backspace => {
                self.compose_input.pop();
                self.compose_selected = 0;
            }
            KeyCode::Char(character) => {
                self.compose_input.push(character);
                self.compose_selected = 0;
            }
            KeyCode::Enter => {
                let typed = self.compose_input.trim().to_owned();
                let picked = self
                    .filtered()
                    .get(self.compose_selected)
                    .map(|component| ((*component).clone(), self.compose_selected));
                if let Some((component, _)) = picked {
                    let step_name = self.unique_step_name(&component.entry.name);
                    self.push_step(
                        component.entry.path,
                        Some(component.contract.hash),
                        step_name,
                        component.contract.role,
                    );
                } else if typed == "import" {
                    self.compose_input.clear();
                    self.compose_error = None;
                    self.compose_stage = ComposeStage::Import;
                } else if typed.is_empty() {
                    self.compose_error = Some("type a name to search, or \"import\"".to_owned());
                } else if scaffold::valid_name(&typed) {
                    self.compose_error = None;
                    self.compose_stage = ComposeStage::UnknownMenu { name: typed };
                } else {
                    self.compose_error = Some(
                        "use 1-64 lowercase letters, digits, `-`, or `_` (or type \"import\")"
                            .to_owned(),
                    );
                }
            }
            _ => {}
        }
    }

    fn compose_key_unknown_menu(&mut self, code: KeyCode) {
        let ComposeStage::UnknownMenu { name } = &self.compose_stage else {
            return;
        };
        let name = name.clone();
        match code {
            KeyCode::Char('1') => {
                self.compose_input.clear();
                self.compose_stage = ComposeStage::Step;
            }
            KeyCode::Char('2') => {
                self.compose_input.clear();
                self.compose_stage = ComposeStage::Import;
            }
            KeyCode::Char('3') => match scaffold_new_component(&name) {
                Ok((path, hash, role)) => {
                    let step_name = self.unique_step_name(&name);
                    self.push_step(path, Some(hash), step_name, role);
                }
                Err(error) => {
                    self.compose_error = Some(error);
                    self.compose_stage = ComposeStage::Step;
                }
            },
            KeyCode::Esc => self.compose_stage = ComposeStage::Step,
            _ => {}
        }
    }

    fn compose_key_import(&mut self, code: KeyCode) {
        match code {
            KeyCode::Esc => {
                self.compose_input.clear();
                self.compose_stage = ComposeStage::Step;
            }
            KeyCode::Backspace => {
                self.compose_input.pop();
            }
            KeyCode::Char(character) => self.compose_input.push(character),
            KeyCode::Enter => {
                let raw = self.compose_input.trim().to_owned();
                let path = PathBuf::from(&raw);
                if !path.is_file() {
                    self.compose_error = Some(format!("`{raw}` was not found"));
                    return;
                }
                match kairo_runtime::detect_contract(&path, Config::default()) {
                    Some(contract) if contract.can_follow(self.compose_role) => {
                        let base = path.file_stem().map_or_else(
                            || "step".to_owned(),
                            |stem| stem.to_string_lossy().into_owned(),
                        );
                        let step_name = self.unique_step_name(&base);
                        self.compose_input.clear();
                        self.push_step(path, Some(contract.hash), step_name, contract.role);
                    }
                    Some(_) => {
                        self.compose_error =
                            Some("this component does not fit the rest of the workflow".to_owned());
                    }
                    None => {
                        self.compose_error =
                            Some("not a supported Kairo Component contract".to_owned());
                    }
                }
            }
            _ => {}
        }
    }

    fn compose_key_add_another(&mut self, code: KeyCode) {
        match code {
            KeyCode::Char('y' | 'Y') => {
                self.refresh_candidates();
                self.compose_stage = ComposeStage::Step;
            }
            KeyCode::Char('n' | 'N') | KeyCode::Enter => {
                let role = self.compose_role.unwrap_or(ComponentRole::ValueStage);
                if role == ComponentRole::StreamOutput {
                    self.compose_input = format!("{}.bin", self.compose_name);
                    self.compose_stage = ComposeStage::OutputFilename;
                } else {
                    self.finalize();
                }
            }
            KeyCode::Esc => self.leave_compose("workflow creation canceled".to_owned()),
            _ => {}
        }
    }

    fn compose_key_output_filename(&mut self, code: KeyCode) {
        match code {
            KeyCode::Backspace => {
                self.compose_input.pop();
            }
            KeyCode::Char(character) => self.compose_input.push(character),
            KeyCode::Enter => {
                self.compose_output_filename = self.compose_input.trim().to_owned();
                self.compose_input = "application/octet-stream".to_owned();
                self.compose_stage = ComposeStage::OutputContentType;
            }
            KeyCode::Esc => self.compose_stage = ComposeStage::AddAnother,
            _ => {}
        }
    }

    fn compose_key_output_content_type(&mut self, code: KeyCode) {
        match code {
            KeyCode::Backspace => {
                self.compose_input.pop();
            }
            KeyCode::Char(character) => self.compose_input.push(character),
            KeyCode::Enter => self.finalize(),
            KeyCode::Esc => self.compose_stage = ComposeStage::OutputFilename,
            _ => {}
        }
    }
}

fn scaffold_new_component(name: &str) -> Result<(PathBuf, ComponentHash, ComponentRole), String> {
    let directory = scaffold::new(name).map_err(|error| error.to_string())?;
    let built = scaffold::build(&directory).map_err(|error| error.to_string())?;
    let contract = kairo_runtime::detect_contract(&built, Config::default())
        .ok_or_else(|| "scaffolded component did not produce a recognizable contract".to_owned())?;
    Ok((built, contract.hash, contract.role))
}
