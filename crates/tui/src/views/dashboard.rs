use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Layout, Rect},
    style::{Color, Style},
    widgets::{Paragraph, Widget, Wrap},
};

use crate::{App, activity};

use super::{centered, empty, label, marker, panel};

pub(crate) fn draw(area: Rect, buffer: &mut Buffer, app: &App) {
    let area = centered(area);
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
    let waiting = app
        .runs
        .iter()
        .filter(|run| activity(run).starts_with("waiting") || activity(run).starts_with("resumes"))
        .count();
    let workers = app.workers.iter().filter(|worker| worker.healthy).count();
    let busy = app.workers.iter().filter(|worker| worker.busy).count();
    let chunks = Layout::vertical([
        Constraint::Length(5),
        Constraint::Length(11),
        Constraint::Min(0),
    ])
    .split(area);
    let metrics = Layout::horizontal([
        Constraint::Percentage(34),
        Constraint::Percentage(33),
        Constraint::Percentage(33),
    ])
    .split(chunks[0]);
    metric(
        metrics[0],
        buffer,
        "Service",
        if app.connected {
            "Connected"
        } else {
            "Local history"
        },
        if app.connected {
            "Updates twice per second"
        } else {
            "Run `kairo start` for persistent activity"
        },
    );
    metric(
        metrics[1],
        buffer,
        "Runs",
        &format!("{} total", app.runs.len()),
        &format!("{queued} queued · {running} running · {waiting} waiting · {completed} completed"),
    );
    metric(
        metrics[2],
        buffer,
        "Workers",
        &format!("{workers} available"),
        &format!("{busy} running · {} known", app.workers.len()),
    );
    let main = Layout::horizontal([Constraint::Percentage(54), Constraint::Percentage(46)])
        .split(chunks[1]);
    selected_summary(main[0], buffer, app);
    recent_runs(main[1], buffer, app);
}

fn metric(area: Rect, buffer: &mut Buffer, title: &str, value: &str, detail: &str) {
    Paragraph::new(format!("{value}\n{detail}"))
        .style(Style::default().fg(Color::White).bg(Color::Black))
        .block(panel(title))
        .wrap(Wrap { trim: true })
        .render(area, buffer);
}

fn selected_summary(area: Rect, buffer: &mut Buffer, app: &App) {
    let Some(run) = app.runs.get(app.selected) else {
        return empty(area, buffer, "No runs yet. Run a workflow first.");
    };
    let components = run.inspection.as_ref().map_or_else(
        || {
            run.stream.as_ref().map_or_else(
                || run.value.as_ref().map_or(0, |item| item.components.len()),
                |item| item.steps.len(),
            )
        },
        |item| item.components.len(),
    );
    let checkpoints = run.inspection.as_ref().map_or_else(
        || {
            run.value.as_ref().map_or(0, |item| {
                item.components
                    .iter()
                    .filter(|component| component.checkpoint.is_some())
                    .count()
            })
        },
        |item| {
            item.components
                .iter()
                .filter(|component| component.checkpoint.is_some())
                .count()
        },
    );
    Paragraph::new(format!(
        "{}\n{}\n\n{} components · {checkpoints} checkpoints\n\nPress Enter for details. Press s to release a selected signal wait.",
        label(run),
        activity(run),
        components,
    ))
    .style(Style::default().fg(Color::White).bg(Color::Black))
    .block(panel("Selected run"))
    .wrap(Wrap { trim: true })
    .render(area, buffer);
}

fn recent_runs(area: Rect, buffer: &mut Buffer, app: &App) {
    let text = if app.runs.is_empty() {
        "Run a workflow to see its activity here.".to_owned()
    } else {
        app.runs
            .iter()
            .take(5)
            .map(|run| format!("{}  {}", marker(run), activity(run)))
            .collect::<Vec<_>>()
            .join("\n")
    };
    Paragraph::new(text)
        .style(Style::default().fg(Color::White).bg(Color::Black))
        .block(panel("Recent activity"))
        .wrap(Wrap { trim: true })
        .render(area, buffer);
}
