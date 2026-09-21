use std::{
    io::{self, IsTerminal},
    path::PathBuf,
    time::{Duration, Instant, SystemTime},
};

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use kairo_core::ComponentHash;
use kairo_runtime::{
    CellInspection, ComponentRole, StreamRunInspection, ValueRunInspection, WorkflowWaitState,
};
use ratatui::DefaultTerminal;
use thiserror::Error;

mod activity;
mod cancel;
pub mod compose;
mod compose_draw;
mod compose_view;
pub mod explain;
mod launch;
mod refresh;
pub mod scaffold;
mod views;

pub(crate) use compose_view::ComposeStage;

pub(crate) use activity::activity;

const REFRESH: Duration = Duration::from_millis(500);

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
    #[error("failed to update workflow state")]
    Control {
        #[source]
        source: kairo_control::ControlError,
    },
}

pub(crate) struct Run {
    pub(crate) name: String,
    pub(crate) path: Option<PathBuf>,
    pub(crate) updated: Option<SystemTime>,
    pub(crate) inspection: Option<CellInspection>,
    pub(crate) stream: Option<StreamRunInspection>,
    pub(crate) value: Option<ValueRunInspection>,
    pub(crate) wait: Option<WorkflowWaitState>,
    pub(crate) service: Option<kairo_control::RunStatus>,
    pub(crate) error: Option<String>,
    pub(crate) history: Vec<kairo_control::RunEvent>,
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) enum Screen {
    Launch,
    Overview,
    Runs,
    Detail,
    Workers,
    Events,
    Compose,
}

impl Screen {
    pub(crate) const ALL: [Self; 6] = [
        Self::Launch,
        Self::Overview,
        Self::Runs,
        Self::Detail,
        Self::Workers,
        Self::Events,
    ];

    pub(crate) fn title(self) -> &'static str {
        match self {
            Self::Launch => "Launch",
            Self::Overview => "Overview",
            Self::Runs => "Runs",
            Self::Detail => "Run detail",
            Self::Workers => "Workers",
            Self::Events => "Events",
            Self::Compose => "New workflow",
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
    pub(crate) notice: Option<String>,
    pub(crate) confirm_signal: Option<String>,
    pub(crate) confirm_cancel: Option<String>,
    pub(crate) catalog: Vec<launch::CatalogEntry>,
    pub(crate) launch_selected: usize,
    pub(crate) launch_input: String,
    pub(crate) launch_editing: bool,
    pub(crate) compose_stage: ComposeStage,
    pub(crate) compose_name: String,
    pub(crate) compose_input: String,
    pub(crate) compose_paths: Vec<PathBuf>,
    pub(crate) compose_hashes: Vec<Option<ComponentHash>>,
    pub(crate) compose_step_names: Vec<String>,
    pub(crate) compose_role: Option<ComponentRole>,
    pub(crate) compose_candidates: Vec<compose::CatalogComponent>,
    pub(crate) compose_selected: usize,
    pub(crate) compose_output_filename: String,
    pub(crate) compose_error: Option<String>,
    // ephemeral processes a submission started; kept alive for the rest of the session (never
    // read again, just held so `Drop` doesn't stop them mid-run -- see `launch::Started`).
    local_service: Option<kairo_control::LocalService>,
    local_effect: Option<kairo_control::LocalEffect>,
}

impl App {
    fn new() -> Result<Self, TuiError> {
        let mut app = Self {
            screen: Screen::Launch,
            selected: 0,
            runs: Vec::new(),
            workers: Vec::new(),
            connected: false,
            help: false,
            dirty: true,
            notice: None,
            confirm_signal: None,
            confirm_cancel: None,
            catalog: launch::catalog(),
            launch_selected: 0,
            launch_input: String::new(),
            launch_editing: false,
            compose_stage: ComposeStage::Name,
            compose_name: String::new(),
            compose_input: String::new(),
            compose_paths: Vec::new(),
            compose_hashes: Vec::new(),
            compose_step_names: Vec::new(),
            compose_role: None,
            compose_candidates: Vec::new(),
            compose_selected: 0,
            compose_output_filename: String::new(),
            compose_error: None,
            local_service: None,
            local_effect: None,
        };
        app.refresh()?;
        Ok(app)
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

    fn request_signal(&mut self) {
        let signal = self
            .runs
            .get(self.selected)
            .and_then(|run| run.service.as_ref())
            .and_then(|status| match status {
                kairo_control::RunStatus::Waiting { reason } => {
                    reason.strip_prefix("signal:").map(str::to_owned)
                }
                _ => None,
            });
        self.notice = Some(match signal {
            Some(signal) => {
                self.confirm_signal = Some(signal.clone());
                format!("send {signal}? press Enter to confirm")
            }
            None => "this run resumes automatically when its timer is ready".to_owned(),
        });
        self.dirty = true;
    }

    fn send_signal(&mut self) -> Result<(), TuiError> {
        let Some(signal) = self.confirm_signal.take() else {
            return Ok(());
        };
        let Some(run) = self.runs.get(self.selected) else {
            return Ok(());
        };
        let endpoint = kairo_control::load_endpoint(std::path::Path::new(".kairo"))
            .map_err(|source| TuiError::Control { source })?;
        kairo_control::signal(&endpoint, run.name.clone(), signal.clone())
            .map_err(|source| TuiError::Control { source })?;
        self.notice = Some(format!("sent {signal}"));
        self.refresh()
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
                if app.launch_editing {
                    match key.code {
                        KeyCode::Enter => app.launch_submit(),
                        KeyCode::Esc => {
                            app.launch_editing = false;
                            app.launch_input.clear();
                        }
                        KeyCode::Backspace => {
                            app.launch_input.pop();
                        }
                        KeyCode::Char(character) => app.launch_input.push(character),
                        _ => {}
                    }
                } else if app.screen == Screen::Compose {
                    app.compose_key(key.code);
                } else {
                    match key.code {
                        KeyCode::Char('q') => return Ok(()),
                        KeyCode::Char('?') => app.help = !app.help,
                        KeyCode::Char('n') if app.screen == Screen::Launch => app.enter_compose(),
                        KeyCode::Char('r' | 'R') => {
                            app.refresh()?;
                            app.notice = Some("refreshed just now".to_owned());
                            refreshed = Instant::now();
                        }
                        KeyCode::Tab => switch_screen(&mut app, 1),
                        KeyCode::BackTab => switch_screen(&mut app, Screen::ALL.len() - 1),
                        KeyCode::Down | KeyCode::Char('j') if app.screen == Screen::Launch => {
                            app.launch_next();
                        }
                        KeyCode::Up | KeyCode::Char('k') if app.screen == Screen::Launch => {
                            app.launch_previous();
                        }
                        KeyCode::Down | KeyCode::Char('j') => app.next(),
                        KeyCode::Up | KeyCode::Char('k') => app.previous(),
                        KeyCode::Char('s') => app.request_signal(),
                        KeyCode::Char('c') => app.request_cancel(),
                        KeyCode::Enter if app.confirm_signal.is_some() => app.send_signal()?,
                        KeyCode::Enter if app.confirm_cancel.is_some() => app.send_cancel()?,
                        KeyCode::Enter if app.screen == Screen::Launch => app.launch_activate(),
                        KeyCode::Enter => app.screen = Screen::Detail,
                        KeyCode::Esc if app.confirm_signal.is_some() => {
                            app.confirm_signal = None;
                            app.notice = Some("signal canceled".to_owned());
                        }
                        KeyCode::Esc if app.confirm_cancel.is_some() => {
                            app.confirm_cancel = None;
                            app.notice = Some("cancel aborted".to_owned());
                        }
                        KeyCode::Esc => app.screen = Screen::Overview,
                        _ => {}
                    }
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
