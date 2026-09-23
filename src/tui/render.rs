// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

//! Responsive Ratatui rendering.

use ratatui::prelude::*;
use ratatui::widgets::{
    Block, BorderType, Borders, List, ListItem, ListState, Paragraph, Tabs, Wrap,
};

mod feed;
mod graphs;
mod layout;
mod metrics;
mod overlays;
mod palette;

use self::feed::render_specific_feed;
use self::graphs::render_traffic;
use self::layout::{
    LayoutMode, WorkspaceDensity, layout_mode, log_rows, overview_cards_height, workspace_columns,
};
use self::metrics::{render_cards, render_overview_page};
use self::overlays::{render_config, render_help};
use super::format;
use super::model::{App, FeedKind, Focus, InstanceView, Lifecycle, Page};

const MIN_WIDTH: u16 = 72;
const MIN_HEIGHT: u16 = 20;

pub fn render(frame: &mut Frame<'_>, app: &App) {
    let area = frame.area();
    if layout_mode(area) == LayoutMode::TooSmall {
        render_too_small(frame, area, app);
        return;
    }

    let [header, body, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(1),
    ])
    .areas(area);
    render_header(frame, header, app);
    match layout_mode(area) {
        LayoutMode::Full => render_full(frame, body, app),
        LayoutMode::Compact => render_compact(frame, body, app),
        LayoutMode::Narrow => render_narrow(frame, body, app),
        LayoutMode::TooSmall => unreachable!("handled above"),
    }
    render_footer(frame, footer, app);

    if app.show_help {
        render_help(frame, area, app);
    }
    if app.show_config {
        render_config(frame, area, app);
    }
}

fn render_header(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let online = app
        .instances
        .iter()
        .filter(|instance| instance.online)
        .count();
    let selected_text = app
        .selected()
        .map(|instance| {
            format!(
                "{} {} #{}",
                instance.meta.role.label(),
                instance.meta.endpoint,
                instance.meta.pid
            )
        })
        .unwrap_or_else(|| "no instance".to_owned());
    let line = if area.width < 80 {
        Line::from(vec![
            Span::styled(
                " NOWHERE ",
                accent(app, Color::Cyan).add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!(
                " {online}/{} instances  ·  ? help ",
                app.instances.len()
            )),
        ])
    } else {
        Line::from(vec![
            Span::styled(
                " NOWHERE ",
                accent(app, Color::Cyan).add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!(" {online}/{} instances  ", app.instances.len())),
            Span::styled(selected_text, Style::default().add_modifier(Modifier::BOLD)),
            Span::raw("  ·  ? help "),
        ])
    };
    frame.render_widget(Paragraph::new(line), area);
}

fn render_footer(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let mut spans = vec![Span::raw(match app.page {
        Page::Overview => " 1 overview  2 logs  ↑↓ select  Tab logs  i config  ? help  q quit ",
        Page::Logs => {
            " 1 overview  2 logs  ↑↓ select/scroll  Tab focus  ←→ pan  Space pause  / filter  q quit "
        }
    })];
    if app.page == Page::Logs && app.paused {
        spans.push(Span::styled(
            " PAUSED ",
            accent(app, Color::Yellow).add_modifier(Modifier::BOLD),
        ));
    }
    if app.page == Page::Logs && (!app.filter.is_empty() || app.filter_editing) {
        let cursor = if app.filter_editing { "█" } else { "" };
        spans.push(Span::styled(
            format!(" /{}{cursor} ", app.filter),
            accent(app, Color::Cyan),
        ));
    }
    if let Some(instance) = app.selected() {
        if instance.dropped_events != 0 {
            spans.push(Span::styled(
                format!(" gap:{} ", instance.dropped_events),
                accent(app, Color::Yellow),
            ));
        }
        if instance.overwritten_events != 0 {
            spans.push(Span::styled(
                format!(" overwritten:{} ", instance.overwritten_events),
                accent(app, Color::Yellow),
            ));
        }
    }
    if let Some(error) = app.global_error.as_deref() {
        spans.push(Span::styled(
            format!(" {} ", format::truncate(error, 40)),
            accent(app, Color::Red),
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn render_full(frame: &mut Frame<'_>, area: Rect, app: &App) {
    render_workspace(frame, area, app, WorkspaceDensity::Full);
}

fn render_compact(frame: &mut Frame<'_>, area: Rect, app: &App) {
    render_workspace(frame, area, app, WorkspaceDensity::Compact);
}

fn render_narrow(frame: &mut Frame<'_>, area: Rect, app: &App) {
    render_workspace(frame, area, app, WorkspaceDensity::Narrow);
}

fn render_workspace(frame: &mut Frame<'_>, area: Rect, app: &App, density: WorkspaceDensity) {
    let [instances, workspace] = workspace_columns(area, density);
    let [tabs, content] =
        Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(workspace);
    render_instances(frame, instances, app);
    render_page_tabs(frame, tabs, app);
    match app.page {
        Page::Overview => render_overview(frame, content, app, density),
        Page::Logs => render_log_page(frame, content, app),
    }
}

fn render_page_tabs(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let labels = Page::ALL
        .into_iter()
        .map(|page| Line::from(format!("{} {}", page_index(page), page.label())))
        .collect::<Vec<_>>();
    let tab = Tabs::new(labels)
        .select(page_index(app.page).saturating_sub(1))
        .divider("·")
        .highlight_style(accent(app, Color::Cyan).add_modifier(Modifier::BOLD))
        .padding(" ", " ");
    frame.render_widget(tab, area);
}

fn render_overview(frame: &mut Frame<'_>, area: Rect, app: &App, density: WorkspaceDensity) {
    if density == WorkspaceDensity::Narrow {
        render_overview_page(frame, area, app);
        return;
    }
    let cards_height = overview_cards_height(area.height, density);
    let [traffic, cards] =
        Layout::vertical([Constraint::Fill(1), Constraint::Length(cards_height)]).areas(area);
    render_traffic(frame, traffic, app);
    render_cards(frame, cards, app);
}

fn render_log_page(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let [access, runtime] = log_rows(area);
    render_specific_feed(frame, access, app, FeedKind::Access);
    render_specific_feed(frame, runtime, app, FeedKind::Runtime);
}

const fn page_index(page: Page) -> usize {
    match page {
        Page::Overview => 1,
        Page::Logs => 2,
    }
}

fn render_instances(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let items = app
        .instances
        .iter()
        .map(|instance| {
            let dot = if instance.online {
                if app.capabilities.unicode { "●" } else { "*" }
            } else if app.capabilities.unicode {
                "○"
            } else {
                "o"
            };
            let status_style = lifecycle_style(app, instance);
            ListItem::new(vec![
                Line::from(vec![
                    Span::styled(format!("{dot} "), status_style),
                    Span::styled(
                        format!(
                            "{} {}",
                            instance.meta.role.short(),
                            format::instance_endpoint(
                                &instance.meta.endpoint,
                                usize::from(area.width.saturating_sub(8))
                            )
                        ),
                        Style::default().add_modifier(Modifier::BOLD),
                    ),
                ]),
                Line::from(vec![
                    Span::raw(format!(
                        "  uid={} #{} ",
                        instance.meta.uid, instance.meta.pid
                    )),
                    Span::styled(instance_status(instance), status_style),
                ]),
            ])
        })
        .collect::<Vec<_>>();
    let block = panel(" INSTANCES ", app.focus == Focus::Instances, app);
    if items.is_empty() {
        frame.render_widget(
            Paragraph::new("No running instances")
                .style(dim(app))
                .block(block)
                .alignment(Alignment::Center),
            area,
        );
        return;
    }
    let list = List::new(items)
        .block(block)
        .highlight_style(accent(app, Color::Cyan).add_modifier(Modifier::REVERSED))
        .highlight_symbol(if app.capabilities.unicode { "▌" } else { ">" });
    let mut state = ListState::default().with_selected(app.selected_index());
    frame.render_stateful_widget(list, area, &mut state);
}

fn render_empty(frame: &mut Frame<'_>, area: Rect, title: &str, message: &str, app: &App) {
    frame.render_widget(
        Paragraph::new(message)
            .alignment(Alignment::Center)
            .style(dim(app))
            .block(panel(title, false, app)),
        area,
    );
}

fn render_too_small(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let message = format!(
        "NOWHERE TUI\n\nTerminal is {}×{}\nMinimum is {MIN_WIDTH}×{MIN_HEIGHT}\n\nResize the terminal or press q",
        area.width, area.height
    );
    frame.render_widget(
        Paragraph::new(message)
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true })
            .style(accent(app, Color::Yellow)),
        area,
    );
}

fn panel<'a>(title: &'a str, focused: bool, app: &App) -> Block<'a> {
    let border_color = if focused {
        Color::Cyan
    } else {
        Color::DarkGray
    };
    Block::default()
        .borders(Borders::ALL)
        .border_type(if app.capabilities.unicode {
            BorderType::Rounded
        } else {
            BorderType::Plain
        })
        .border_style(accent(app, border_color))
        .title(title)
}

fn instance_status(instance: &InstanceView) -> String {
    if instance.online {
        instance.lifecycle.label().to_owned()
    } else {
        "OFFLINE".to_owned()
    }
}

fn lifecycle_style(app: &App, instance: &InstanceView) -> Style {
    let color = if !instance.online {
        palette::FAILURE
    } else {
        match instance.lifecycle {
            Lifecycle::Ready => palette::SUCCESS,
            Lifecycle::Starting | Lifecycle::Draining => palette::WARNING,
            Lifecycle::Stopped | Lifecycle::Failed => palette::FAILURE,
            Lifecycle::Unknown | Lifecycle::Other(_) => Color::DarkGray,
        }
    };
    accent(app, color)
}

fn accent(app: &App, color: Color) -> Style {
    if app.capabilities.color {
        Style::default().fg(color)
    } else {
        Style::default()
    }
}

fn dim(app: &App) -> Style {
    if app.capabilities.color {
        Style::default().fg(Color::Gray)
    } else {
        Style::default().add_modifier(Modifier::DIM)
    }
}

#[cfg(test)]
#[path = "../tests/tui/render.rs"]
mod tests;
