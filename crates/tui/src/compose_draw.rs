use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    widgets::{List, ListItem, Paragraph, Widget, Wrap},
};

use crate::{App, compose, compose_view::ComposeStage, views::panel};

pub(crate) fn draw(area: Rect, buffer: &mut Buffer, app: &App) {
    let chunks = Layout::vertical([Constraint::Min(3), Constraint::Length(1)]).split(area);
    let chain = if app.compose_step_names.is_empty() {
        "(no steps yet)".to_owned()
    } else {
        app.compose_step_names.join(" -> ")
    };
    let error = app
        .compose_error
        .as_deref()
        .map_or_else(String::new, |error| format!("\nerror: {error}"));

    match &app.compose_stage {
        ComposeStage::Name => {
            let text = format!("workflow name: {}█{error}", app.compose_input);
            Paragraph::new(text)
                .style(Style::default().fg(Color::White).bg(Color::Black))
                .block(panel("New workflow"))
                .wrap(Wrap { trim: true })
                .render(chunks[0], buffer);
        }
        ComposeStage::Step => {
            let columns = Layout::horizontal([Constraint::Percentage(52), Constraint::Min(30)])
                .split(chunks[0]);
            let filtered = app.filtered();
            let items: Vec<ListItem> = filtered
                .iter()
                .enumerate()
                .map(|(index, component)| {
                    let style = if index == app.compose_selected {
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::White)
                    };
                    ListItem::new(compose::describe(component)).style(style)
                })
                .collect();
            let title = if app.compose_candidates.is_empty() {
                "compatible Components · none found".to_owned()
            } else {
                format!("compatible Components · {}", filtered.len())
            };
            List::new(items)
                .style(Style::default().fg(Color::White).bg(Color::Black))
                .block(panel(&title))
                .render(columns[0], buffer);
            let text = format!(
                "steps so far\n  {chain}\n\nsearch or type a name (or \"import\"): {}█{error}\n\n↑↓ select · Enter add",
                app.compose_input
            );
            Paragraph::new(text)
                .style(Style::default().fg(Color::White).bg(Color::Black))
                .block(panel(&app.compose_name))
                .wrap(Wrap { trim: true })
                .render(columns[1], buffer);
        }
        ComposeStage::Import => {
            let text = format!(
                "steps so far\n  {chain}\n\ncomponent path: {}█{error}",
                app.compose_input
            );
            Paragraph::new(text)
                .style(Style::default().fg(Color::White).bg(Color::Black))
                .block(panel(&app.compose_name))
                .wrap(Wrap { trim: true })
                .render(chunks[0], buffer);
        }
        ComposeStage::AddAnother => {
            let text = format!("steps so far\n  {chain}\n\nadd another? y/N{error}");
            Paragraph::new(text)
                .style(Style::default().fg(Color::White).bg(Color::Black))
                .block(panel(&app.compose_name))
                .wrap(Wrap { trim: true })
                .render(chunks[0], buffer);
        }
        ComposeStage::OutputFilename => {
            let text = format!("output filename: {}█{error}", app.compose_input);
            Paragraph::new(text)
                .style(Style::default().fg(Color::White).bg(Color::Black))
                .block(panel(&app.compose_name))
                .wrap(Wrap { trim: true })
                .render(chunks[0], buffer);
        }
        ComposeStage::OutputContentType => {
            let text = format!("output content type: {}█{error}", app.compose_input);
            Paragraph::new(text)
                .style(Style::default().fg(Color::White).bg(Color::Black))
                .block(panel(&app.compose_name))
                .wrap(Wrap { trim: true })
                .render(chunks[0], buffer);
        }
    }
    Paragraph::new("Esc cancel · Enter confirm · type to search/answer")
        .style(Style::default().fg(Color::Gray).bg(Color::Black))
        .render(chunks[1], buffer);
}
