use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::Line,
    widgets::Widget,
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Tabs, Wrap},
};

use crate::{App, Run, Screen, activity};

mod dashboard;
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
        Screen::Overview => dashboard::draw(chunks[1], buffer, app),
        Screen::Runs => runs(chunks[1], buffer, app),
        Screen::Detail => detail(chunks[1], buffer, app),
        Screen::Workers => workers(chunks[1], buffer, app),
        Screen::Events => events(chunks[1], buffer, app),
    }
    Paragraph::new("↑↓ select · Enter details · r refresh · Tab switch · ? help · q quit")
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
fn runs(area: Rect, buffer: &mut Buffer, app: &App) {
    let area = centered(area);
    let chunks = Layout::horizontal([Constraint::Percentage(52), Constraint::Min(36)]).split(area);
    let visible = usize::from(area.height.saturating_sub(2)).clamp(1, 12);
    let (start, end) = visible_runs(app.runs.len(), app.selected, visible);
    let title = if app.runs.is_empty() {
        "Runs · none yet".to_owned()
    } else {
        format!(
            "Runs · {}–{} of {} · newest first",
            start + 1,
            end,
            app.runs.len()
        )
    };
    let items: Vec<_> = app
        .runs
        .iter()
        .enumerate()
        .skip(start)
        .take(end - start)
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
        .block(panel(&title))
        .render(chunks[0], buffer);
    detail(chunks[1], buffer, app);
}
fn visible_runs(total: usize, selected: usize, visible: usize) -> (usize, usize) {
    let start = selected.saturating_sub(visible / 2);
    let end = total.min(start.saturating_add(visible));
    let start = end.saturating_sub(visible);
    (start, end)
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
    let mut lines = vec![format!(
        "{}\n{}\nrun id · {}",
        label(run),
        activity(run),
        run.name
    )];
    if let Some(kairo_control::RunStatus::Completed { output, worker }) = &run.service {
        lines.push(if worker.is_empty() {
            format!("output · {output}")
        } else {
            format!("worker · {worker}\noutput · {output}")
        });
    }
    if let Some(inspection) = &run.inspection {
        lines.push(format!("input · {}", inspection.input));
        for component in &inspection.components {
            if component.index > 0 {
                lines.push("  ↓".to_owned());
            }
            let state = component.output.map_or_else(
                || "running".to_owned(),
                |output| format!("completed · output {output}"),
            );
            let duration = component
                .duration_us
                .map(format_duration)
                .map_or_else(String::new, |value| format!(" · {value}"));
            lines.push(format!("{} · {state}{duration}", component.name));
            if let Some(true) = component.durable_after {
                lines.push(match &component.checkpoint {
                    Some(_) => "  └─ checkpoint saved".to_owned(),
                    None => "  └─ checkpoint pending".to_owned(),
                });
            }
        }
        if let Some(path) = &run.path
            && let Ok(receipts) = kairo_runtime::inspect_receipts(path)
        {
            for receipt in receipts {
                lines.push(format!(
                    "effect · {} · {}",
                    receipt.operation, receipt.status
                ));
            }
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
    let area = centered(area);
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
    let area = centered(area);
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

pub(crate) fn empty(area: Rect, buffer: &mut Buffer, message: &str) {
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
    Paragraph::new("Keyboard shortcuts\n\nTab / Shift-Tab  change screen\n↑↓ or j/k       select a run\nEnter            open details\nr                 refresh now\nEsc              overview\nq                quit")
        .style(Style::default().fg(Color::White).bg(Color::Black))
        .block(panel("Help"))
        .wrap(Wrap { trim: true })
        .render(popup, buffer);
}

pub(crate) fn panel(title: &str) -> Block<'_> {
    Block::default()
        .title(format!(" {title} "))
        .borders(Borders::ALL)
        .style(Style::default().fg(Color::Gray).bg(Color::Black))
}

pub(crate) fn centered(area: Rect) -> Rect {
    let width = area.width.min(160);
    Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y,
        width,
        height: area.height,
    }
}

fn format_duration(microseconds: u64) -> String {
    if microseconds >= 1_000_000 {
        format!(
            "{}.{:03}s",
            microseconds / 1_000_000,
            microseconds % 1_000_000 / 1_000
        )
    } else if microseconds >= 1_000 {
        format!("{}.{:03}ms", microseconds / 1_000, microseconds % 1_000)
    } else {
        format!("{microseconds}µs")
    }
}

pub(crate) fn label(run: &Run) -> &str {
    run.inspection
        .as_ref()
        .and_then(|inspection| inspection.name.as_deref())
        .unwrap_or(&run.name)
}

pub(crate) fn marker(run: &Run) -> String {
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
