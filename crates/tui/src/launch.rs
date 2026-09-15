use std::{
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use kairo_core::{Config, IoInput, IoOutput, Workflow, WorkflowMode};
use kairo_runtime::Runtime;

use crate::App;

impl App {
    pub(crate) fn launch_next(&mut self) {
        if !self.catalog.is_empty() {
            self.launch_selected = (self.launch_selected + 1) % self.catalog.len();
            self.dirty = true;
        }
    }

    pub(crate) fn launch_previous(&mut self) {
        if !self.catalog.is_empty() {
            self.launch_selected = self
                .launch_selected
                .checked_sub(1)
                .unwrap_or(self.catalog.len() - 1);
            self.dirty = true;
        }
    }

    /// starts editing this workflow's input, or submits right away when it needs none.
    pub(crate) fn launch_activate(&mut self) {
        let Some(entry) = self.catalog.get(self.launch_selected) else {
            return;
        };
        if needs_text_input(&entry.workflow) {
            self.launch_editing = true;
            self.launch_input.clear();
        } else {
            self.launch_submit();
        }
        self.dirty = true;
    }

    pub(crate) fn launch_submit(&mut self) {
        let Some(entry) = self.catalog.get(self.launch_selected) else {
            return;
        };
        self.launch_editing = false;
        self.notice = Some(format!("running {}…", entry.workflow.name()));
        self.dirty = true;
        let (result, started) = submit(entry, &self.launch_input);
        // an ephemeral local service/effect service (spawned as this process's children) must
        // outlive this call -- the run keeps going after `submit` returns, and dropping either
        // guard here would kill it mid-run. Kept alive for the rest of the TUI session; a later
        // submission that finds one already running gets `None` here and changes nothing.
        if let Some(service) = started.service {
            self.local_service = Some(service);
        }
        if let Some(effect) = started.effect {
            self.local_effect = Some(effect);
        }
        match result {
            SubmitResult::Completed(output) => {
                self.notice = Some(format!("{} completed · {output}", entry.workflow.name()));
            }
            SubmitResult::Queued(message) => self.notice = Some(message),
            SubmitResult::Failed(message) => self.notice = Some(format!("failed: {message}")),
        }
        self.launch_input.clear();
        let _ = self.refresh();
    }
}

pub(crate) struct CatalogEntry {
    pub(crate) path: PathBuf,
    pub(crate) workflow: Workflow,
}

/// the same scanning primitive `kairo workflows` uses (`kairo_core::discover`), with the same
/// curation rule: bundled demos need a description, a project's own `workflows/` never does.
pub(crate) fn catalog() -> Vec<CatalogEntry> {
    let config = Config::default();
    let curated = kairo_core::discover(Path::new("demos/reference"), config)
        .into_iter()
        .filter(|found| found.workflow.description().is_some());
    let project = kairo_core::discover(Path::new("workflows"), config);
    let mut entries: Vec<_> = curated
        .chain(project)
        .map(|found| CatalogEntry {
            path: found.path,
            workflow: found.workflow,
        })
        .collect();
    entries.sort_by(|left, right| left.workflow.name().cmp(right.workflow.name()));
    entries
}

pub(crate) fn needs_text_input(workflow: &Workflow) -> bool {
    matches!(workflow.io().input, IoInput::File | IoInput::Value)
}

pub(crate) enum SubmitResult {
    Completed(String),
    Queued(String),
    Failed(String),
}

const DEFAULT_TUI_WORKERS: usize = 2;

/// any ephemeral local process this submission started, which the caller must keep alive for as
/// long as the run needs it -- dropping either guard kills the process it guards.
#[derive(Default)]
pub(crate) struct Started {
    pub(crate) service: Option<kairo_control::LocalService>,
    pub(crate) effect: Option<kairo_control::LocalEffect>,
}

/// runs (or enqueues) a real workflow, using the exact same primitives `kairo run` uses:
/// `kairo_runtime::Runtime` directly for a fully local run, or `kairo_control::ensure_endpoint`
/// plus `submit_run` for one that needs the control plane. That is the same lifecycle helper
/// `kairo run --watch` uses, so the TUI never needs `kairo up` running first and never
/// reimplements any of that lifecycle logic itself.
///
/// A workflow needing durable artifact storage -- which the TUI doesn't resolve project storage
/// config for in this version -- is declined with a clear message rather than guessed at.
pub(crate) fn submit(entry: &CatalogEntry, input: &str) -> (SubmitResult, Started) {
    match entry.workflow.mode() {
        WorkflowMode::Value => (submit_value(entry, input), Started::default()),
        WorkflowMode::Scalar => submit_scalar(entry),
        WorkflowMode::Stream => (unsupported("stream workflows"), Started::default()),
    }
}

fn unsupported(what: &str) -> SubmitResult {
    SubmitResult::Failed(format!(
        "{what} aren't runnable from the TUI yet -- use `kairo run`"
    ))
}

fn load(entry: &CatalogEntry) -> Result<(Runtime, Workflow), String> {
    let runtime = Runtime::new(Config::default()).map_err(|error| error.to_string())?;
    let workflow = runtime
        .load_workflow(&entry.path)
        .map_err(|error| error.to_string())?;
    Ok((runtime, workflow))
}

fn block_on<F: std::future::Future>(future: F) -> Result<F::Output, String> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map(|runtime| runtime.block_on(future))
        .map_err(|error| error.to_string())
}

fn submit_value(entry: &CatalogEntry, input: &str) -> SubmitResult {
    if entry.workflow.requires_durable_artifacts() || entry.workflow.has_unresolved_durability() {
        return unsupported("workflows needing durable storage");
    }
    let (runtime, workflow) = match load(entry) {
        Ok(loaded) => loaded,
        Err(message) => return SubmitResult::Failed(message),
    };
    match block_on(runtime.run_value_workflow(&workflow, input.as_bytes().to_vec())) {
        Ok(Ok(result)) => SubmitResult::Completed(describe_output(&workflow, &result.output)),
        Ok(Err(error)) => SubmitResult::Failed(error.to_string()),
        Err(message) => SubmitResult::Failed(message),
    }
}

fn submit_scalar(entry: &CatalogEntry) -> (SubmitResult, Started) {
    if entry.workflow.requires_durable_artifacts() || entry.workflow.has_unresolved_durability() {
        return (
            unsupported("workflows needing durable storage"),
            Started::default(),
        );
    }
    let (runtime, workflow) = match load(entry) {
        Ok(loaded) => loaded,
        Err(message) => return (SubmitResult::Failed(message), Started::default()),
    };
    if workflow.wait().is_none() && workflow.effect().is_none() {
        let result = match block_on(runtime.run_workflow(&workflow)) {
            Ok(Ok(result)) => SubmitResult::Completed(result.output.to_string()),
            Ok(Err(error)) => SubmitResult::Failed(error.to_string()),
            Err(message) => SubmitResult::Failed(message),
        };
        return (result, Started::default());
    }
    let effect = match kairo_control::ensure_effect_service(&workflow) {
        Ok(effect) => effect,
        Err(error) => return (SubmitResult::Failed(error.to_string()), Started::default()),
    };
    let (endpoint, service) =
        match kairo_control::ensure_endpoint(Path::new(".kairo"), None, DEFAULT_TUI_WORKERS, false)
        {
            Ok(ready) => ready,
            Err(error) => {
                return (
                    SubmitResult::Failed(error.to_string()),
                    Started {
                        service: None,
                        effect: Some(effect),
                    },
                );
            }
        };
    let state = generated_state_path(&workflow);
    let result = match kairo_control::submit_run(&endpoint, &workflow, &entry.path, &state, None) {
        Ok(id) => SubmitResult::Queued(format!("queued {id} -- see it on the Runs screen")),
        Err(error) => SubmitResult::Failed(error.to_string()),
    };
    (
        result,
        Started {
            service,
            effect: Some(effect),
        },
    )
}

fn describe_output(workflow: &Workflow, bytes: &[u8]) -> String {
    let shown = match std::str::from_utf8(bytes) {
        Ok(text) => text.to_owned(),
        Err(_) => format!("<{} bytes (binary)>", bytes.len()),
    };
    match workflow.io().output {
        IoOutput::Artifact => format!("{shown} (use `kairo run` with --output to export bytes)"),
        _ => shown,
    }
}

fn generated_state_path(workflow: &Workflow) -> PathBuf {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    Path::new(".kairo").join(format!("tui-{}-{suffix}.db", workflow.name()))
}
