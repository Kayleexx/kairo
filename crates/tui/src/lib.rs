use std::{io, io::IsTerminal, time::Duration};

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use kairo_runtime::{CellInspection, CellStatus, discover_cells, inspect_cell};
use ratatui::{
    DefaultTerminal,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::Line,
    widgets::Widget,
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Tabs, Wrap},
};
use thiserror::Error;

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

struct Run {
    name: String,
    path: std::path::PathBuf,
    inspection: Option<CellInspection>,
    error: Option<String>,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Screen {
    Overview,
    Runs,
    Detail,
    Workers,
    Events,
}

impl Screen {
    const ALL: [Self; 5] = [
        Self::Overview,
        Self::Runs,
        Self::Detail,
        Self::Workers,
        Self::Events,
    ];

    fn title(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Runs => "Runs",
            Self::Detail => "Run detail",
            Self::Workers => "Workers",
            Self::Events => "Events",
        }
    }
}

struct App {
    screen: Screen,
    selected: usize,
    runs: Vec<Run>,
    workers: Vec<kairo_control::WorkerSnapshot>,
    connected: bool,
    help: bool,
    dirty: bool,
}

impl App {
    fn new() -> Result<Self, TuiError> {
        let mut app = Self {
            screen: Screen::Overview,
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
        if let Ok(endpoint) = kairo_control::load_endpoint(std::path::Path::new(".kairo"))
            && let Ok(snapshot) = kairo_control::snapshot(&endpoint)
        {
            self.connected = true;
            self.workers = snapshot.workers;
        }
        let cells = discover_cells().map_err(|source| TuiError::State { source })?;
        self.runs = cells
            .into_iter()
            .map(|cell| match inspect_cell(&cell.path) {
                Ok(inspection) => Run {
                    name: cell.name,
                    path: cell.path,
                    inspection: Some(inspection),
                    error: None,
                },
                Err(error) => Run {
                    name: cell.name,
                    path: cell.path,
                    inspection: None,
                    error: Some(error.to_string()),
                },
            })
            .collect();
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
    loop {
        if app.dirty {
            terminal
                .draw(|frame| draw(frame.area(), frame.buffer_mut(), &app))
                .map_err(|source| TuiError::Terminal { source })?;
            app.dirty = false;
        }
        if !event::poll(REFRESH).map_err(|source| TuiError::Terminal { source })? {
            app.refresh()?;
            continue;
        }
        let event = event::read().map_err(|source| TuiError::Terminal { source })?;
        let Event::Key(key) = event else {
            app.dirty = true;
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        match key.code {
            KeyCode::Char('q') => return Ok(()),
            KeyCode::Char('?') => app.help = !app.help,
            KeyCode::Tab => {
                let index = Screen::ALL
                    .iter()
                    .position(|screen| *screen == app.screen)
                    .unwrap_or(0);
                app.screen = Screen::ALL[(index + 1) % Screen::ALL.len()];
            }
            KeyCode::BackTab => {
                let index = Screen::ALL
                    .iter()
                    .position(|screen| *screen == app.screen)
                    .unwrap_or(0);
                app.screen = Screen::ALL[index.checked_sub(1).unwrap_or(Screen::ALL.len() - 1)];
            }
            KeyCode::Down | KeyCode::Char('j') => app.next(),
            KeyCode::Up | KeyCode::Char('k') => app.previous(),
            KeyCode::Enter => app.screen = Screen::Detail,
            KeyCode::Esc => app.screen = Screen::Overview,
            _ => {}
        }
        app.dirty = true;
    }
}

fn draw(area: Rect, buffer: &mut ratatui::buffer::Buffer, app: &App) {
    let chunks = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(3),
        Constraint::Length(1),
    ])
    .split(area);
    let titles = Screen::ALL.map(|screen| Line::from(screen.title()));
    Tabs::new(titles)
        .select(
            Screen::ALL
                .iter()
                .position(|screen| *screen == app.screen)
                .unwrap_or(0),
        )
        .highlight_style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
        .block(Block::default().borders(Borders::BOTTOM))
        .render(chunks[0], buffer);
    match app.screen {
        Screen::Overview => overview(chunks[1], buffer, app),
        Screen::Runs => runs(chunks[1], buffer, app),
        Screen::Detail => detail(chunks[1], buffer, app),
        Screen::Workers => workers(chunks[1], buffer, app),
        Screen::Events => events(chunks[1], buffer, app),
    }
    Paragraph::new("Tab switch · ↑↓ select · Enter details · ? help · q quit")
        .style(Style::default().fg(Color::DarkGray))
        .render(chunks[2], buffer);
    if app.help {
        help(area, buffer);
    }
}

fn overview(area: Rect, buffer: &mut ratatui::buffer::Buffer, app: &App) {
    let completed = app
        .runs
        .iter()
        .filter(|run| {
            matches!(
                run.inspection.as_ref().map(|value| &value.status),
                Some(CellStatus::Completed { .. })
            )
        })
        .count();
    let active = app.runs.len().saturating_sub(completed);
    Paragraph::new(format!(
        "{}\n\n{} completed · {} active or recoverable\n{} workers available\n\n{}",
        if app.connected {
            "Live service connected"
        } else {
            "Showing local history"
        },
        completed,
        active,
        app.workers.iter().filter(|worker| worker.healthy).count(),
        if app.connected {
            "Updates refresh automatically."
        } else {
            "Start a live service with `kairo start`, then open this screen with `kairo tui`."
        }
    ))
    .block(Block::default().title("Status").borders(Borders::ALL))
    .wrap(Wrap { trim: true })
    .render(area, buffer);
}

fn runs(area: Rect, buffer: &mut ratatui::buffer::Buffer, app: &App) {
    let items: Vec<_> = app
        .runs
        .iter()
        .enumerate()
        .map(|(index, run)| {
            let selected = index == app.selected;
            let text = match (&run.inspection, &run.error) {
                (Some(inspection), _) => format!("{} · {}", run.name, status(&inspection.status)),
                (None, Some(error)) => format!("{} · unavailable · {error}", run.name),
                (None, None) => run.name.clone(),
            };
            ListItem::new(text).style(if selected {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            })
        })
        .collect();
    List::new(items)
        .block(Block::default().title("Runs").borders(Borders::ALL))
        .render(area, buffer);
}

fn detail(area: Rect, buffer: &mut ratatui::buffer::Buffer, app: &App) {
    let Some(run) = app.runs.get(app.selected) else {
        return empty(area, buffer, "No runs yet. Run a workflow first.");
    };
    let text = match &run.inspection {
        Some(inspection) => {
            let mut lines = vec![format!("{} · {}", run.name, status(&inspection.status))];
            for component in &inspection.components {
                let state = component.output.map_or("running", |_| "completed");
                lines.push(format!("{} · {}", component.name, state));
            }
            lines.join("\n")
        }
        None => format!(
            "{}\n{}",
            run.name,
            run.error.as_deref().unwrap_or("unavailable")
        ),
    };
    Paragraph::new(text)
        .block(Block::default().title("Run detail").borders(Borders::ALL))
        .wrap(Wrap { trim: true })
        .render(area, buffer);
}

fn workers(area: Rect, buffer: &mut ratatui::buffer::Buffer, app: &App) {
    let text = if app.workers.is_empty() {
        "No live workers. Run `kairo start` to create a local service.".to_owned()
    } else {
        app.workers
            .iter()
            .map(|worker| {
                format!(
                    "{} · {} · {}",
                    worker.id,
                    if worker.healthy {
                        "available"
                    } else {
                        "disconnected"
                    },
                    if worker.busy { "running" } else { "idle" }
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    Paragraph::new(text)
        .block(Block::default().title("Workers").borders(Borders::ALL))
        .render(area, buffer);
}

fn events(area: Rect, buffer: &mut ratatui::buffer::Buffer, app: &App) {
    let text = app
        .runs
        .get(app.selected)
        .map(|run| {
            kairo_runtime::inspect_events(&run.path, 0, 200)
                .map(|events| {
                    events
                        .into_iter()
                        .map(|event| format!("{:>4}  {}", event.sequence, event.summary))
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .unwrap_or_else(|error| error.to_string())
        })
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| "Select a run to view its recorded events.".to_owned());
    Paragraph::new(text)
        .block(
            Block::default()
                .title("Recorded events")
                .borders(Borders::ALL),
        )
        .render(area, buffer);
}

fn empty(area: Rect, buffer: &mut ratatui::buffer::Buffer, message: &str) {
    Paragraph::new(message)
        .block(Block::default().title("Run detail").borders(Borders::ALL))
        .render(area, buffer);
}
fn help(area: Rect, buffer: &mut ratatui::buffer::Buffer) {
    let popup = Rect {
        x: area.width / 8,
        y: area.height / 4,
        width: area.width.saturating_mul(3) / 4,
        height: area.height / 2,
    };
    Clear.render(popup, buffer);
    Paragraph::new("Keyboard shortcuts\n\nTab / Shift-Tab  change screen\n↑↓ or j/k       select a run\nEnter            open details\nEsc              overview\nq                quit")
        .block(Block::default().title("Help").borders(Borders::ALL))
        .wrap(Wrap { trim: true })
        .render(popup, buffer);
}

fn status(status: &CellStatus) -> &'static str {
    match status {
        CellStatus::Completed { .. } => "completed",
        CellStatus::Ready { .. } => "ready",
        CellStatus::Interrupted { .. } => "interrupted",
        CellStatus::CheckpointPending { .. } => "saving checkpoint",
        CellStatus::Finalizing => "finalizing",
    }
}
