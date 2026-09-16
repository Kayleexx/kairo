use std::path::{Path, PathBuf};

use crate::{Config, Workflow};

const MAX_FILES: usize = 256;

pub struct DiscoveredWorkflow {
    pub path: PathBuf,
    pub workflow: Workflow,
}

/// recursively finds every valid workflow document under `directory` -- the one shared scanning
/// primitive any front end (CLI, TUI) builds its own catalog/curation rules on top of. A missing
/// directory or an unparseable file is skipped, never an error.
pub fn discover(directory: &Path, config: Config) -> Vec<DiscoveredWorkflow> {
    let mut found = Vec::new();
    let mut remaining = MAX_FILES;
    walk(directory, &mut found, &mut remaining, config);
    found
}

// bundled demos are curated (need a `description`); a project's own workflows/ is not.
pub fn catalog(config: Config) -> Vec<DiscoveredWorkflow> {
    let curated = discover(Path::new("demos/reference"), config)
        .into_iter()
        .filter(|found| found.workflow.description().is_some());
    let project = discover(Path::new("workflows"), config);
    let mut entries: Vec<_> = curated.chain(project).collect();
    entries.sort_by(|left, right| left.workflow.name().cmp(right.workflow.name()));
    entries
}

fn walk(
    directory: &Path,
    found: &mut Vec<DiscoveredWorkflow>,
    remaining: &mut usize,
    config: Config,
) {
    if *remaining == 0 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        *remaining = remaining.saturating_sub(1);
        let path = entry.path();
        if path.is_dir() {
            walk(&path, found, remaining, config);
        } else if is_yaml(&path)
            && let Ok(workflow) =
                Workflow::load(&path, config.max_workflow_bytes, config.max_workflow_steps)
        {
            found.push(DiscoveredWorkflow { path, workflow });
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
