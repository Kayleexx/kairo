use std::{
    fs,
    path::{Path, PathBuf},
};

use kairo_core::{Config, Workflow};
use thiserror::Error;

const MAX_FILES: usize = 256;

pub(crate) struct ReferenceWorkflow {
    pub(crate) name: String,
    pub(crate) accepts: Vec<String>,
    pub(crate) result: String,
}

#[derive(Debug, Error)]
pub(crate) enum DiscoveryError {
    #[error("workflow name `{name}` is ambiguous; use one of: {paths}")]
    Ambiguous { name: String, paths: String },
}

pub(crate) fn resolve(path: &Path, config: Config) -> Result<PathBuf, DiscoveryError> {
    if path.exists() || is_yaml(path) {
        return Ok(path.to_path_buf());
    }
    let sibling = PathBuf::from(format!("{}.yaml", path.display()));
    if sibling.is_file() {
        return Ok(sibling);
    }
    let name = path.to_string_lossy();
    let mut matches = Vec::new();
    let mut remaining = MAX_FILES;
    for root in [Path::new("workflows"), Path::new("demos")] {
        collect(root, &mut matches, &mut remaining, config, name.as_ref());
    }
    match matches.len() {
        1 => Ok(matches.remove(0)),
        0 => Ok(path.to_path_buf()),
        _ => Err(DiscoveryError::Ambiguous {
            name: name.into_owned(),
            paths: matches
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", "),
        }),
    }
}

pub(crate) fn references(config: Config) -> Vec<ReferenceWorkflow> {
    let mut entries = Vec::new();
    let mut remaining = MAX_FILES;
    collect_references(
        Path::new("demos/reference"),
        &mut entries,
        &mut remaining,
        config,
    );
    entries.sort_by(|left, right| left.name.cmp(&right.name));
    entries
}

pub(crate) fn print_references(config: Config) {
    let references = references(config);
    if references.is_empty() {
        return;
    }
    println!("WORKFLOW   INPUT       RESULT");
    for reference in references {
        let input = if reference.accepts.is_empty() {
            "none".to_owned()
        } else {
            reference
                .accepts
                .iter()
                .map(|value| {
                    if value == "mp4-h264" {
                        "h264 mp4"
                    } else {
                        value
                    }
                })
                .collect::<Vec<_>>()
                .join("/")
        };
        println!("{:<10} {:<11} {}", reference.name, input, reference.result);
    }
    println!();
}

fn collect_references(
    directory: &Path,
    references: &mut Vec<ReferenceWorkflow>,
    remaining: &mut usize,
    config: Config,
) {
    if *remaining == 0 {
        return;
    }
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        *remaining = remaining.saturating_sub(1);
        let path = entry.path();
        if path.is_dir() {
            collect_references(&path, references, remaining, config);
        } else if is_yaml(&path)
            && let Ok(workflow) =
                Workflow::load(&path, config.max_workflow_bytes, config.max_workflow_steps)
            && workflow.description().is_some()
        {
            references.push(ReferenceWorkflow {
                name: workflow.name().to_owned(),
                accepts: workflow.accepts().to_vec(),
                result: reference_result(&workflow),
            });
        }
        if *remaining == 0 {
            return;
        }
    }
}

fn reference_result(workflow: &Workflow) -> String {
    if let Some(output) = workflow.output() {
        return output.filename.clone();
    }
    if workflow.effect().is_some() {
        return "external action".to_owned();
    }
    if let Some(wait) = workflow.wait() {
        return match wait {
            kairo_core::WorkflowWait::Signal(_) => "approval wait".to_owned(),
            kairo_core::WorkflowWait::Timer(_) => "delayed result".to_owned(),
        };
    }
    "analysis".to_owned()
}

fn collect(
    directory: &Path,
    matches: &mut Vec<PathBuf>,
    remaining: &mut usize,
    config: Config,
    name: &str,
) {
    if *remaining == 0 {
        return;
    }
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        *remaining = remaining.saturating_sub(1);
        let path = entry.path();
        if path.is_dir() {
            collect(&path, matches, remaining, config, name);
        } else if is_yaml(&path)
            && Workflow::load(&path, config.max_workflow_bytes, config.max_workflow_steps)
                .is_ok_and(|workflow| workflow.matches_name(name))
        {
            matches.push(path);
        }
        if *remaining == 0 {
            return;
        }
    }
}

fn is_yaml(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("yaml") || extension.eq_ignore_ascii_case("yml")
        })
}
