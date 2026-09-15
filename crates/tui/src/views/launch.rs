use kairo_core::IoInput;
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, List, ListItem, Paragraph, Widget, Wrap},
};

use crate::App;

use super::{centered, empty, panel};

pub(crate) fn draw(area: Rect, buffer: &mut Buffer, app: &App) {
    let area = centered(area);
    if app.catalog.is_empty() {
        return empty(
            area,
            buffer,
            "No workflows found. Add one under workflows/, or see the bundled demos.",
        );
    }
    let chunks = Layout::horizontal([Constraint::Percentage(42), Constraint::Min(30)]).split(area);
    let items: Vec<ListItem> = app
        .catalog
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            let style = if index == app.launch_selected {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };
            ListItem::new(entry.workflow.name().to_owned()).style(style)
        })
        .collect();
    List::new(items)
        .block(
            Block::default()
                .title(" Workflows ")
                .borders(Borders::ALL)
                .style(Style::default().fg(Color::DarkGray).bg(Color::Black)),
        )
        .render(chunks[0], buffer);
    detail(chunks[1], buffer, app);
}

fn detail(area: Rect, buffer: &mut Buffer, app: &App) {
    let Some(entry) = app.catalog.get(app.launch_selected) else {
        return empty(area, buffer, "Select a workflow.");
    };
    let workflow = &entry.workflow;
    let description = workflow.description().unwrap_or("(no description set)");
    let accepts = if workflow.accepts().is_empty() {
        "anything".to_owned()
    } else {
        workflow.accepts().join(", ")
    };
    let produces = if workflow.produces().is_empty() {
        "unspecified".to_owned()
    } else {
        workflow.produces().join(", ")
    };
    let prompt = if app.launch_editing {
        format!("\nvalue: {}█", app.launch_input)
    } else {
        match workflow.io().input {
            IoInput::None => "\nneeds no input -- press Enter to run".to_owned(),
            _ => "\npress Enter to provide a value and run".to_owned(),
        }
    };
    Paragraph::new(format!(
        "{description}\n\naccepts: {accepts}\nproduces: {produces}{prompt}"
    ))
    .style(Style::default().fg(Color::White).bg(Color::Black))
    .block(panel(workflow.name()))
    .wrap(Wrap { trim: true })
    .render(area, buffer);
}
