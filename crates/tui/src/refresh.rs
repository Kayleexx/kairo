use std::{collections::BTreeMap, fs};

use kairo_runtime::{
    CellStatus, StreamRunStatus, discover_cells, inspect_cell, inspect_stream_run,
    inspect_workflow_wait,
};

use crate::{App, Run, TuiError};

impl App {
    pub(crate) fn refresh(&mut self) -> Result<(), TuiError> {
        let selected_name = self.runs.get(self.selected).map(|run| run.name.clone());
        self.connected = false;
        self.workers.clear();
        let mut service_runs = BTreeMap::new();
        if let Ok(endpoint) = kairo_control::load_endpoint(std::path::Path::new(".kairo"))
            && let Ok(snapshot) = kairo_control::snapshot(&endpoint)
        {
            self.connected = true;
            self.workers = snapshot.workers;
            service_runs = snapshot
                .runs
                .into_iter()
                .map(|run| (run.id, (run.status, run.history)))
                .collect();
        }
        let cells = discover_cells().map_err(|source| TuiError::State { source })?;
        let mut runs: Vec<_> = cells
            .into_iter()
            .map(|cell| {
                let updated = fs::metadata(&cell.path)
                    .and_then(|metadata| metadata.modified())
                    .ok();
                match inspect_stream_run(&cell.path) {
                    Ok(Some(stream)) => Run {
                        name: cell.name,
                        path: Some(cell.path),
                        updated,
                        inspection: None,
                        stream: Some(stream),
                        wait: None,
                        service: None,
                        error: None,
                        history: Vec::new(),
                    },
                    Ok(None) => match (inspect_cell(&cell.path), inspect_workflow_wait(&cell.path))
                    {
                        (Ok(inspection), Ok(wait)) => Run {
                            name: cell.name,
                            path: Some(cell.path),
                            updated,
                            inspection: Some(inspection),
                            stream: None,
                            wait,
                            service: None,
                            error: None,
                            history: Vec::new(),
                        },
                        (Err(error), _) => Run {
                            name: cell.name,
                            path: Some(cell.path),
                            updated,
                            inspection: None,
                            stream: None,
                            wait: None,
                            service: None,
                            error: Some(error.to_string()),
                            history: Vec::new(),
                        },
                        (_, Err(error)) => Run {
                            name: cell.name,
                            path: Some(cell.path),
                            updated,
                            inspection: None,
                            stream: None,
                            wait: None,
                            service: None,
                            error: Some(error.to_string()),
                            history: Vec::new(),
                        },
                    },
                    Err(error) => Run {
                        name: cell.name,
                        path: Some(cell.path),
                        updated,
                        inspection: None,
                        stream: None,
                        wait: None,
                        service: None,
                        error: Some(error.to_string()),
                        history: Vec::new(),
                    },
                }
            })
            .collect();
        for run in &mut runs {
            if let Some((status, history)) = service_runs.remove(&run.name) {
                run.service = Some(status);
                run.history = history;
            }
        }
        runs.extend(
            service_runs
                .into_iter()
                .map(|(name, (status, history))| Run {
                    name,
                    path: None,
                    updated: None,
                    inspection: None,
                    stream: None,
                    wait: None,
                    service: Some(status),
                    error: None,
                    history,
                }),
        );
        runs.sort_by(|left, right| {
            run_rank(left)
                .cmp(&run_rank(right))
                .then_with(|| right.updated.cmp(&left.updated))
                .then_with(|| left.name.cmp(&right.name))
        });
        self.selected = selected_name
            .and_then(|name| runs.iter().position(|run| run.name == name))
            .unwrap_or(0)
            .min(runs.len().saturating_sub(1));
        self.runs = runs;
        self.dirty = true;
        Ok(())
    }
}

fn run_rank(run: &Run) -> u8 {
    match run.service.as_ref() {
        Some(kairo_control::RunStatus::Queued | kairo_control::RunStatus::Running { .. }) => 0,
        Some(kairo_control::RunStatus::Failed { .. }) => 1,
        _ if run.error.is_some() => 1,
        _ if run.inspection.as_ref().is_some_and(|inspection| {
            !matches!(inspection.status, CellStatus::Completed { .. })
        }) =>
        {
            1
        }
        _ if run
            .stream
            .as_ref()
            .is_some_and(|stream| !matches!(stream.status, StreamRunStatus::Completed)) =>
        {
            1
        }
        _ => 2,
    }
}
