use std::{
    collections::BTreeMap,
    io,
    io::IsTerminal,
    path::PathBuf,
    time::{Duration, Instant},
};

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use kairo_runtime::{CellInspection, CellStatus, discover_cells, inspect_cell};
use ratatui::DefaultTerminal;
use thiserror::Error;

mod views;

const REFRESH: Duration = Duration::from_millis(200);

#[derive(Debug, Error)]
pub enum TuiError {
    #[error("`kairo tui` needs an interactive terminal")]
    NotTerminal,
    #[error("terminal I/O failed")]
    Terminal {
        #[source]
        source: io::Error,
    },
    #[error("failed to read local run state")]
    State {
        #[source]
        source: kairo_runtime::LocalStateError,
    },
}

pub(crate) struct Run {
    pub(crate) name: String,
    pub(crate) path: Option<PathBuf>,
    pub(crate) inspection: Option<CellInspection>,
    pub(crate) service: Option<kairo_control::RunStatus>,
    pub(crate) error: Option<String>,
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) enum Screen {
    Overview,
    Runs,
    Detail,
    Workers,
    Events,
}

impl Screen {
    pub(crate) const ALL: [Self; 5] = [
        Self::Overview,
        Self::Runs,
        Self::Detail,
        Self::Workers,
        Self::Events,
    ];

    pub(crate) fn title(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Runs => "Runs",
            Self::Detail => "Run detail",
            Self::Workers => "Workers",
            Self::Events => "Events",
        }
    }
}

pub(crate) struct App {
    pub(crate) screen: Screen,
    pub(crate) selected: usize,
    pub(crate) runs: Vec<Run>,
    pub(crate) workers: Vec<kairo_control::WorkerSnapshot>,
    pub(crate) connected: bool,
    pub(crate) help: bool,
    pub(crate) dirty: bool,
}

impl App {
    fn new() -> Result<Self, TuiError> {
        let mut app = Self {
            screen: Screen::Runs,
            selected: 0,
            runs: Vec::new(),
            workers: Vec::new(),
            connected: false,
            help: false,
            dirty: true,
        };
        app.refresh()?;
        Ok(app)
    }

    fn refresh(&mut self) -> Result<(), TuiError> {
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
                .map(|run| (run.id, run.status))
                .collect();
        }
        let cells = discover_cells().map_err(|source| TuiError::State { source })?;
        let mut runs: BTreeMap<_, _> = cells
            .into_iter()
            .map(|cell| match inspect_cell(&cell.path) {
                Ok(inspection) => Run {
                    name: cell.name,
                    path: Some(cell.path),
                    inspection: Some(inspection),
                    service: None,
                    error: None,
                },
                Err(error) => Run {
                    name: cell.name,
                    path: Some(cell.path),
                    inspection: None,
                    service: None,
                    error: Some(error.to_string()),
                },
            })
            .map(|run| (run.name.clone(), run))
            .collect();
        for (name, status) in service_runs {
            if let Some(run) = runs.get_mut(&name) {
                run.service = Some(status);
            } else {
                runs.insert(
                    name.clone(),
                    Run {
                        name,
                        path: None,
                        inspection: None,
                        service: Some(status),
                        error: None,
                    },
                );
            }
        }
        self.runs = runs.into_values().collect();
        self.selected = self.selected.min(self.runs.len().saturating_sub(1));
        self.dirty = true;
        Ok(())
    }

    fn next(&mut self) {
        if !self.runs.is_empty() {
            self.selected = (self.selected + 1) % self.runs.len();
            self.dirty = true;
        }
    }

    fn previous(&mut self) {
        if !self.runs.is_empty() {
            self.selected = self.selected.checked_sub(1).unwrap_or(self.runs.len() - 1);
            self.dirty = true;
        }
    }
}

pub fn run() -> Result<(), TuiError> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err(TuiError::NotTerminal);
    }
    let mut terminal = ratatui::try_init().map_err(|source| TuiError::Terminal { source })?;
    let result = run_app(&mut terminal);
    let restored = ratatui::try_restore().map_err(|source| TuiError::Terminal { source });
    result.and(restored)
}

fn run_app(terminal: &mut DefaultTerminal) -> Result<(), TuiError> {
    let mut app = App::new()?;
    let mut refreshed = Instant::now();
    loop {
        if app.dirty {
            terminal
                .draw(|frame| views::draw(frame.area(), frame.buffer_mut(), &app))
                .map_err(|source| TuiError::Terminal { source })?;
            app.dirty = false;
        }
        let wait = REFRESH.saturating_sub(refreshed.elapsed());
        if event::poll(wait).map_err(|source| TuiError::Terminal { source })? {
            let event = event::read().map_err(|source| TuiError::Terminal { source })?;
            if let Event::Key(key) = event
                && key.kind == KeyEventKind::Press
            {
                match key.code {
                    KeyCode::Char('q') => return Ok(()),
                    KeyCode::Char('?') => app.help = !app.help,
                    KeyCode::Char('r' | 'R') => {
                        app.refresh()?;
                        refreshed = Instant::now();
                    }
                    KeyCode::Tab => switch_screen(&mut app, 1),
                    KeyCode::BackTab => switch_screen(&mut app, Screen::ALL.len() - 1),
                    KeyCode::Down | KeyCode::Char('j') => app.next(),
                    KeyCode::Up | KeyCode::Char('k') => app.previous(),
                    KeyCode::Enter => app.screen = Screen::Detail,
                    KeyCode::Esc => app.screen = Screen::Overview,
                    _ => {}
                }
            }
            app.dirty = true;
        }
        if refreshed.elapsed() >= REFRESH {
            app.refresh()?;
            refreshed = Instant::now();
        }
    }
}

fn switch_screen(app: &mut App, offset: usize) {
    let index = Screen::ALL
        .iter()
        .position(|screen| *screen == app.screen)
        .unwrap_or(0);
    app.screen = Screen::ALL[(index + offset) % Screen::ALL.len()];
}

pub(crate) fn activity(run: &Run) -> String {
    match run.service.as_ref() {
        Some(kairo_control::RunStatus::Queued) => "queued".to_owned(),
        Some(kairo_control::RunStatus::Running { worker }) => format!("running · {worker}"),
        Some(kairo_control::RunStatus::Failed { message }) => format!("failed · {message}"),
        Some(kairo_control::RunStatus::Completed { output }) if run.inspection.is_none() => {
            format!("completed · output {output}")
        }
        _ => run.inspection.as_ref().map_or_else(
            || "waiting for worker".to_owned(),
            |inspection| status(&inspection.status).to_owned(),
        ),
    }
}

pub(crate) fn status(status: &CellStatus) -> &'static str {
    match status {
        CellStatus::Completed { .. } => "completed",
        CellStatus::Ready { .. } => "ready",
        CellStatus::Interrupted { .. } => "interrupted",
        CellStatus::CheckpointPending { .. } => "saving checkpoint",
        CellStatus::Finalizing => "finalizing",
    }
}
