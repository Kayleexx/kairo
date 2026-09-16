use std::path::{Path, PathBuf};

use kairo_core::{Config, Workflow};
use thiserror::Error;

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
    let matches: Vec<_> = [Path::new("workflows"), Path::new("demos")]
        .iter()
        .flat_map(|root| kairo_core::discover(root, config))
        .filter(|found| found.workflow.matches_name(&name))
        .map(|found| found.path)
        .collect();
    match matches.len() {
        1 => Ok(matches.into_iter().next().unwrap_or_default()),
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
    kairo_core::catalog(config)
        .into_iter()
        .map(|found| ReferenceWorkflow {
            name: found.workflow.name().to_owned(),
            accepts: found.workflow.accepts().to_vec(),
            result: reference_result(&found.workflow),
        })
        .collect()
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

fn is_yaml(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("yaml") || extension.eq_ignore_ascii_case("yml")
        })
}
