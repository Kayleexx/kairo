use std::{
    fs,
    time::{Duration, SystemTime},
};

use kairo_runtime::{CellStatus, StreamRunStatus};

use crate::state::{self, LocalCell};

use super::InspectionError;

pub(crate) struct PruneOptions {
    pub(crate) older_than_hours: Option<u64>,
    pub(crate) workflow: Option<String>,
    pub(crate) yes: bool,
}

pub(crate) fn prune(options: PruneOptions) -> Result<(), InspectionError> {
    let inventory = super::inventory::load()?;
    let cutoff = options
        .older_than_hours
        .map(|hours| Duration::from_secs(hours.saturating_mul(3600)));

    let mut candidates: Vec<&LocalCell> = Vec::new();
    for (run, inspection) in &inventory.ready {
        if matches!(inspection.status, CellStatus::Completed { .. })
            && matches_workflow(inspection.name.as_deref(), options.workflow.as_deref())
        {
            candidates.push(run);
        }
    }
    for (run, inspection) in &inventory.streams {
        if matches!(
            inspection.status,
            StreamRunStatus::Completed | StreamRunStatus::Failed(_)
        ) && matches_workflow(Some(&inspection.workflow), options.workflow.as_deref())
        {
            candidates.push(run);
        }
    }

    if candidates.is_empty() {
        println!("nothing to prune");
        return Ok(());
    }

    let mut removed = 0;
    let mut skipped = 0;
    for run in candidates {
        if let Some(cutoff) = cutoff {
            let age = state::modified(run)
                .ok()
                .and_then(|modified| SystemTime::now().duration_since(modified).ok());
            if age.is_none_or(|age| age < cutoff) {
                continue;
            }
        }
        if !lock_is_free(run) {
            println!("  ○ {} · in use, skipped", run.name);
            skipped += 1;
            continue;
        }
        if options.yes {
            remove_run(run)?;
            println!("  ✓ {} · removed", run.name);
        } else {
            println!("  · {} · would remove", run.name);
        }
        removed += 1;
    }

    if options.yes {
        println!("\n{removed} run(s) removed, {skipped} in use and skipped");
    } else {
        println!("\n{removed} run(s) would be removed; pass --yes to delete");
    }
    Ok(())
}

fn matches_workflow(name: Option<&str>, requested: Option<&str>) -> bool {
    match requested {
        None => true,
        Some(requested) => name == Some(requested),
    }
}

fn lock_is_free(run: &LocalCell) -> bool {
    let lock_path = run.path.with_extension("lock");
    let Ok(file) = fs::File::create(&lock_path) else {
        return false;
    };
    file.try_lock().is_ok()
}

fn remove_run(run: &LocalCell) -> Result<(), InspectionError> {
    let mut journals = vec![run.path.clone()];
    journals.extend(super::sibling_groups(&run.path));
    for journal in journals {
        for extension in ["db", "db-shm", "db-wal", "lock"] {
            let path = journal.with_extension(extension);
            match fs::remove_file(&path) {
                Ok(()) => {}
                Err(source) if source.kind() == std::io::ErrorKind::NotFound => {}
                Err(source) => return Err(InspectionError::Prune { path, source }),
            }
        }
    }
    Ok(())
}
