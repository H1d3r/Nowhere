// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

//! Responsive Ratatui rendering.

use ratatui::prelude::*;
use ratatui::widgets::{
    Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph, Tabs, Wrap,
};

mod feed;
mod graphs;
mod metrics;
mod palette;

use self::feed::render_specific_feed;
use self::graphs::render_traffic;
#[cfg(test)]
use self::graphs::{
    CONNECTION_GRAPH_STYLE, GraphStyle, PROCESS_GRAPH_STYLE, TRAFFIC_COLORS, TRAFFIC_GRAPH_STYLE,
    rate_series, traffic_cells,
};
use self::metrics::{render_cards, render_overview_page};
use super::format;
use super::model::{App, FeedKind, Focus, InstanceView, Lifecycle, Page};

const MIN_WIDTH: u16 = 72;
const MIN_HEIGHT: u16 = 20;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LayoutMode {
    Full,
    Compact,
    Narrow,
    TooSmall,
}

pub const fn layout_mode(area: Rect) -> LayoutMode {
    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        LayoutMode::TooSmall
    } else if area.width >= 120 && area.height >= 32 {
        LayoutMode::Full
    } else if area.width >= 80 && area.height >= 28 {
        LayoutMode::Compact
    } else {
        LayoutMode::Narrow
    }
}

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
        Page::Overview => " 1 overview  2 logs  ↑↓ select  Tab logs  ? help  q quit ",
        Page::Logs => {
            " 1 overview  2 logs  ↑↓ select/scroll  Tab focus  ←→ pan  Space pause  / filter  p privacy  q quit "
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WorkspaceDensity {
    Full,
    Compact,
    Narrow,
}

impl WorkspaceDensity {
    const fn sidebar_width(self) -> u16 {
        match self {
            Self::Full => 25,
            Self::Compact => 23,
            Self::Narrow => 20,
        }
    }
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

fn workspace_columns(area: Rect, density: WorkspaceDensity) -> [Rect; 2] {
    Layout::horizontal([
        Constraint::Length(density.sidebar_width()),
        Constraint::Fill(1),
    ])
    .areas(area)
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

fn overview_cards_height(height: u16, density: WorkspaceDensity) -> u16 {
    match density {
        WorkspaceDensity::Full => ((height * 2) / 5).clamp(11, 18),
        WorkspaceDensity::Compact => ((height * 2) / 5).clamp(10, 15),
        WorkspaceDensity::Narrow => height,
    }
}

fn render_log_page(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let [access, runtime] = log_rows(area);
    render_specific_feed(frame, access, app, FeedKind::Access);
    render_specific_feed(frame, runtime, app, FeedKind::Runtime);
}

fn log_rows(area: Rect) -> [Rect; 2] {
    Layout::vertical([Constraint::Ratio(1, 2), Constraint::Ratio(1, 2)]).areas(area)
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
                            format::truncate(&instance.meta.endpoint, 14)
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

fn render_help(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let width = area.width.saturating_sub(4).min(76);
    let height = area.height.saturating_sub(2).min(22);
    let popup = centered(area, width, height);
    frame.render_widget(Clear, popup);
    let text = vec![
        Line::from("Navigation").style(accent(app, Color::Cyan).add_modifier(Modifier::BOLD)),
        Line::from("  ↑↓ / jk       select instance or scroll events"),
        Line::from("  ←→ / hl       move instance / pan focused log"),
        Line::from("  Tab / BackTab focus Instances / Access / Runtime"),
        Line::from("  1 / 2         Overview / Logs"),
        Line::from(""),
        Line::from("Views").style(accent(app, Color::Cyan).add_modifier(Modifier::BOLD)),
        Line::from("  Space         pause or resume live tail"),
        Line::from("  PgUp / PgDn   scroll ten records"),
        Line::from("  /             filter both logs"),
        Line::from("  c             clear focused local log"),
        Line::from("  p             reveal/mask client addresses locally"),
        Line::from("  q / Ctrl-C    quit"),
        Line::from(""),
        Line::from(
            "The TUI is read-only. Charts and feeds begin when this TUI connects and are not persisted.",
        )
        .style(dim(app)),
    ];
    frame.render_widget(
        Paragraph::new(text).wrap(Wrap { trim: true }).block(
            panel(" HELP · Esc/? to close ", true, app).border_style(accent(app, Color::Cyan)),
        ),
        popup,
    );
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    Rect::new(
        area.x + area.width.saturating_sub(width) / 2,
        area.y + area.height.saturating_sub(height) / 2,
        width.min(area.width),
        height.min(area.height),
    )
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
        Style::default().fg(Color::DarkGray)
    } else {
        Style::default().add_modifier(Modifier::DIM)
    }
}

#[cfg(test)]
#[path = "../tests/tui/render.rs"]
mod tests;
