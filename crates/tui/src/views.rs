use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::Line,
    widgets::Widget,
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Tabs, Wrap},
};

use crate::{App, Run, Screen, activity};

pub(crate) fn draw(area: Rect, buffer: &mut Buffer, app: &App) {
    Block::default()
        .style(Style::default().bg(Color::Black))
        .render(area, buffer);
    let chunks = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(3),
        Constraint::Length(1),
    ])
    .split(area);
    let titles = Screen::ALL.map(|screen| Line::from(screen.title()));
    Tabs::new(titles)
        .select(screen_index(app.screen))
        .highlight_style(
            Style::default()
                .fg(Color::Cyan)
                .bg(Color::Black)
                .add_modifier(Modifier::BOLD),
        )
        .style(Style::default().fg(Color::White).bg(Color::Black))
        .block(
            Block::default()
                .title(" KAIRO · LIVE ACTIVITY ")
                .borders(Borders::BOTTOM)
                .style(Style::default().fg(Color::DarkGray).bg(Color::Black)),
        )
        .render(chunks[0], buffer);
    match app.screen {
        Screen::Overview => overview(chunks[1], buffer, app),
        Screen::Runs => runs(chunks[1], buffer, app),
        Screen::Detail => detail(chunks[1], buffer, app),
        Screen::Workers => workers(chunks[1], buffer, app),
        Screen::Events => events(chunks[1], buffer, app),
    }
    Paragraph::new("↑↓ select · Enter full details · Tab switch screen · ? help · q quit")
        .style(Style::default().fg(Color::Gray).bg(Color::Black))
        .render(chunks[2], buffer);
    if app.help {
        help(area, buffer);
    }
}

fn screen_index(screen: Screen) -> usize {
    Screen::ALL
        .iter()
        .position(|item| *item == screen)
        .unwrap_or(0)
}

fn overview(area: Rect, buffer: &mut Buffer, app: &App) {
    let queued = app
        .runs
        .iter()
        .filter(|run| activity(run) == "queued")
        .count();
    let running = app
        .runs
        .iter()
        .filter(|run| activity(run).starts_with("running"))
        .count();
    let completed = app
        .runs
        .iter()
        .filter(|run| activity(run).starts_with("completed"))
        .count();
    Paragraph::new(format!(
        "{}\n\n{queued} queued · {running} running · {completed} completed\n{} workers available\n\n{}",
        if app.connected {
            "Live service connected"
        } else {
            "Showing local history"
        },
        app.workers.iter().filter(|worker| worker.healthy).count(),
        if app.connected {
            "Updates refresh automatically."
        } else {
            "Start a live service with `kairo start`, then open `kairo tui`."
        }
    ))
    .style(Style::default().fg(Color::White).bg(Color::Black))
    .block(panel("Activity"))
    .wrap(Wrap { trim: true })
    .render(area, buffer);
}

fn runs(area: Rect, buffer: &mut Buffer, app: &App) {
    let chunks = Layout::horizontal([Constraint::Percentage(52), Constraint::Min(36)]).split(area);
    let items: Vec<_> = app
        .runs
        .iter()
        .enumerate()
        .map(|(index, run)| {
            let text = match &run.error {
                Some(error) => format!("{}  unavailable · {error}", label(run)),
                None => format!("{}  {}", marker(run), activity(run)),
            };
            ListItem::new(text).style(selected(index, app.selected))
        })
        .collect();
    List::new(items)
        .style(Style::default().fg(Color::White).bg(Color::Black))
        .block(panel(&format!("Recent runs · {}", app.runs.len())))
        .render(chunks[0], buffer);
    detail(chunks[1], buffer, app);
}

fn selected(index: usize, current: usize) -> Style {
    if index == current {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White).bg(Color::Black)
    }
}

fn detail(area: Rect, buffer: &mut Buffer, app: &App) {
    let Some(run) = app.runs.get(app.selected) else {
        return empty(area, buffer, "No runs yet. Run a workflow first.");
    };
    let mut lines = vec![format!("{}\n{}", label(run), activity(run))];
    if let Some(inspection) = &run.inspection {
        for component in &inspection.components {
            let state = component.output.map_or("running", |_| "completed");
            lines.push(format!("{} · {state}", component.name));
        }
    } else if run.error.is_none() {
        lines.push("Waiting for local progress to be recorded.".to_owned());
    }
    Paragraph::new(lines.join("\n"))
        .style(Style::default().fg(Color::White).bg(Color::Black))
        .block(panel("Run detail"))
        .wrap(Wrap { trim: true })
        .render(area, buffer);
}

fn workers(area: Rect, buffer: &mut Buffer, app: &App) {
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
        .style(Style::default().fg(Color::White).bg(Color::Black))
        .block(panel("Workers"))
        .render(area, buffer);
}

fn events(area: Rect, buffer: &mut Buffer, app: &App) {
    let text = app
        .runs
        .get(app.selected)
        .and_then(events_for)
        .unwrap_or_else(|| "Progress will appear after a worker starts the run.".to_owned());
    Paragraph::new(text)
        .style(Style::default().fg(Color::White).bg(Color::Black))
        .block(panel("Recorded events"))
        .render(area, buffer);
}

fn events_for(run: &Run) -> Option<String> {
    let path = run.path.as_ref()?;
    kairo_runtime::inspect_events(path, 0, 200)
        .map(|events| {
            events
                .into_iter()
                .map(|event| format!("{:>4}  {}", event.sequence, event.summary))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .ok()
        .filter(|text| !text.is_empty())
}

fn empty(area: Rect, buffer: &mut Buffer, message: &str) {
    Paragraph::new(message)
        .style(Style::default().fg(Color::White).bg(Color::Black))
        .block(panel("Run detail"))
        .render(area, buffer);
}

fn help(area: Rect, buffer: &mut Buffer) {
    let popup = Rect {
        x: area.width / 8,
        y: area.height / 4,
        width: area.width.saturating_mul(3) / 4,
        height: area.height / 2,
    };
    Clear.render(popup, buffer);
    Paragraph::new("Keyboard shortcuts\n\nTab / Shift-Tab  change screen\n↑↓ or j/k       select a run\nEnter            open details\nEsc              overview\nq                quit")
        .style(Style::default().fg(Color::White).bg(Color::Black))
        .block(panel("Help"))
        .wrap(Wrap { trim: true })
        .render(popup, buffer);
}

fn panel(title: &str) -> Block<'_> {
    Block::default()
        .title(format!(" {title} "))
        .borders(Borders::ALL)
        .style(Style::default().fg(Color::Gray).bg(Color::Black))
}

fn label(run: &Run) -> &str {
    run.inspection
        .as_ref()
        .and_then(|inspection| inspection.name.as_deref())
        .unwrap_or(&run.name)
}

fn marker(run: &Run) -> String {
    let state = activity(run);
    let symbol = if state.starts_with("completed") {
        "✓"
    } else if state.starts_with("failed") || state.contains("interrupted") {
        "!"
    } else if state.starts_with("queued") {
        "○"
    } else {
        "●"
    };
    format!("{symbol} {}", label(run))
}
