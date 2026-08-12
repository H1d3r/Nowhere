// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

//! Traffic charts, metric cards, and sparklines.

use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

use crate::tui::format;
use crate::tui::model::App;

use super::graphs::{render_dual_sparkline, render_sparkline, scaled_data, spark_data};
use super::{accent, dim, instance_status, lifecycle_style, palette, panel, render_empty};

pub(super) fn render_cards(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let [selected, connections, carriers] = Layout::horizontal([
        Constraint::Ratio(1, 3),
        Constraint::Ratio(1, 3),
        Constraint::Ratio(1, 3),
    ])
    .areas(area);
    render_selected_card(frame, selected, app);
    render_connections_card(frame, connections, app);
    render_carriers_card(frame, carriers, app);
}

pub(super) fn render_overview_page(frame: &mut Frame<'_>, area: Rect, app: &App) {
    if area.width >= 96 {
        let [left, right] =
            Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
                .areas(area);
        let [selected, connections] =
            Layout::vertical([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(left);
        render_selected_card(frame, selected, app);
        render_connections_card(frame, connections, app);
        render_carriers_card(frame, right, app);
    } else {
        let [selected, lower] =
            Layout::vertical([Constraint::Length(7), Constraint::Fill(1)]).areas(area);
        let [connections, carriers] =
            Layout::horizontal([Constraint::Ratio(1, 2), Constraint::Ratio(1, 2)]).areas(lower);
        render_selected_card(frame, selected, app);
        render_connections_card(frame, connections, app);
        render_carriers_card(frame, carriers, app);
    }
}

fn render_selected_card(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let Some(instance) = app.selected() else {
        render_empty(frame, area, " SELECTED ", "No instance selected", app);
        return;
    };
    let snapshot = instance.snapshot.as_ref();
    let uptime = snapshot
        .map(|snapshot| format::duration_ms(snapshot.uptime_ms))
        .unwrap_or_else(|| "—".to_owned());
    let content_width = usize::from(area.width.saturating_sub(2));
    let config_width = content_width.saturating_sub(4);
    let config_rows = usize::from(area.height.saturating_sub(2))
        .saturating_sub(4)
        .min(4);
    let config_lines = wrap_tokens(&instance.meta.config_summary, config_width, config_rows);
    let mut lines = vec![
        Line::from(vec![
            Span::styled(
                instance.meta.role.label(),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(instance_status(instance), lifecycle_style(app, instance)),
            Span::raw("  up "),
            Span::styled(uptime, accent(app, palette::UP)),
        ]),
        Line::from(vec![
            Span::styled("PID ", dim(app)),
            Span::raw(instance.meta.pid.to_string()),
            Span::styled("  UID ", dim(app)),
            Span::raw(instance.meta.uid.to_string()),
        ]),
        Line::from(vec![
            Span::styled("VER ", dim(app)),
            Span::raw(&instance.meta.version),
            Span::styled("  SMP ", dim(app)),
            Span::raw(format!("{}ms", instance.meta.telemetry_interval_ms)),
        ]),
        Line::from(vec![
            Span::styled("LST ", dim(app)),
            Span::raw(format::truncate(
                &instance.meta.endpoint,
                content_width.saturating_sub(4),
            )),
        ]),
    ];
    for (index, config) in config_lines.into_iter().enumerate() {
        lines.push(
            Line::from(vec![
                Span::styled(if index == 0 { "CFG " } else { "    " }, dim(app)),
                Span::raw(config),
            ])
            .style(dim(app)),
        );
    }
    frame.render_widget(
        Paragraph::new(lines).block(panel(" SELECTED ", false, app)),
        area,
    );
}

fn render_connections_card(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let Some(instance) = app.selected() else {
        render_empty(frame, area, " CONNECTIONS ", "—", app);
        return;
    };
    let snapshot = instance.snapshot.as_ref();
    let rate = instance.latest_history();
    let active = [
        (
            "TCP",
            snapshot
                .map(|value| value.tcp_active.max(0).to_string())
                .unwrap_or_else(|| "—".to_owned()),
            palette::TCP,
        ),
        (
            "UDP",
            snapshot
                .map(|value| value.udp_active.max(0).to_string())
                .unwrap_or_else(|| "—".to_owned()),
            palette::UDP,
        ),
    ];
    let throughput = [
        (
            if app.capabilities.unicode {
                "↑"
            } else {
                "UP"
            },
            format::bits_per_second(rate.upload_bps),
            palette::UP,
        ),
        (
            if app.capabilities.unicode {
                "↓"
            } else {
                "DN"
            },
            format::bits_per_second(rate.download_bps),
            palette::DOWN,
        ),
    ];
    let totals = [
        (
            if app.capabilities.unicode {
                "Σ↑"
            } else {
                "TUP"
            },
            snapshot
                .map(|value| format::bytes(value.upload_bytes()))
                .unwrap_or_else(|| "—".to_owned()),
            palette::UP,
        ),
        (
            if app.capabilities.unicode {
                "Σ↓"
            } else {
                "TDN"
            },
            snapshot
                .map(|value| format::bytes(value.download_bytes()))
                .unwrap_or_else(|| "—".to_owned()),
            palette::DOWN,
        ),
    ];
    let block = panel(" CONNECTIONS ", false, app);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let [metrics, history] =
        Layout::vertical([Constraint::Length(3), Constraint::Fill(1)]).areas(inner);
    let rows = Layout::vertical([Constraint::Length(1); 3]).split(metrics);
    render_metric_row(frame, rows[0], &active, app);
    render_metric_row(frame, rows[1], &throughput, app);
    render_metric_row(frame, rows[2], &totals, app);
    render_dual_sparkline(
        frame,
        history,
        spark_data(&instance.history, history.width, |point| {
            point.tcp_active.max(0) as u64
        }),
        palette::TCP,
        spark_data(&instance.history, history.width, |point| {
            point.udp_active.max(0) as u64
        }),
        palette::UDP,
        app,
    );
}

fn render_carriers_card(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let Some(instance) = app.selected() else {
        render_empty(frame, area, " CARRIERS / PROCESS ", "—", app);
        return;
    };
    let snapshot = instance.snapshot.as_ref();
    let block = panel(" CARRIERS / PROCESS ", false, app);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let rows = Layout::vertical([Constraint::Ratio(1, 3); 3]).split(inner);
    render_graph_pair(
        frame,
        rows[0],
        MetricGraph::new(
            "TLS",
            snapshot
                .map(|value| value.link_tcp.to_string())
                .unwrap_or_else(|| "—".to_owned()),
            spark_data(&instance.history, rows[0].width / 2, |point| {
                point.tls_links
            }),
            palette::TLS,
        ),
        MetricGraph::new(
            "QUIC",
            snapshot
                .map(|value| value.link_udp.to_string())
                .unwrap_or_else(|| "—".to_owned()),
            spark_data(&instance.history, rows[0].width / 2, |point| {
                point.quic_links
            }),
            palette::QUIC,
        ),
        app,
    );
    render_graph_pair(
        frame,
        rows[1],
        MetricGraph::new(
            "PING",
            snapshot
                .map(|value| format!("{}ms", value.ping_ms))
                .unwrap_or_else(|| "—".to_owned()),
            spark_data(&instance.history, rows[1].width / 2, |point| point.ping_ms),
            palette::PING,
        ),
        MetricGraph::new(
            "POOL",
            snapshot
                .map(|value| value.pool_active.to_string())
                .unwrap_or_else(|| "—".to_owned()),
            spark_data(&instance.history, rows[1].width / 2, |point| {
                point.pool_active
            }),
            palette::POOL,
        ),
        app,
    );
    render_graph_pair(
        frame,
        rows[2],
        MetricGraph::new(
            "CPU",
            snapshot
                .and_then(|snapshot| snapshot.cpu_percent)
                .map(|value| format!("{value:.1}%"))
                .unwrap_or_else(|| "—".to_owned()),
            scaled_data(&instance.history, rows[2].width / 2, |point| {
                point.cpu_percent
            }),
            palette::CPU,
        ),
        MetricGraph::new(
            "RSS",
            snapshot
                .and_then(|snapshot| snapshot.rss_bytes)
                .map(format::bytes)
                .unwrap_or_else(|| "—".to_owned()),
            spark_data(&instance.history, rows[2].width / 2, |point| {
                point.rss_bytes
            }),
            palette::RSS,
        ),
        app,
    );
}

struct MetricGraph {
    label: &'static str,
    current: String,
    data: Vec<u64>,
    color: Color,
}

impl MetricGraph {
    const fn new(label: &'static str, current: String, data: Vec<u64>, color: Color) -> Self {
        Self {
            label,
            current,
            data,
            color,
        }
    }
}

fn render_graph_pair(
    frame: &mut Frame<'_>,
    area: Rect,
    first: MetricGraph,
    second: MetricGraph,
    app: &App,
) {
    let [left, right] = Layout::horizontal([Constraint::Ratio(1, 2), Constraint::Ratio(1, 2)])
        .spacing(1)
        .areas(area);
    render_labeled_graph(
        frame,
        left,
        first.label,
        first.current,
        first.data,
        first.color,
        app,
    );
    render_labeled_graph(
        frame,
        right,
        second.label,
        second.current,
        second.data,
        second.color,
        app,
    );
}

fn render_metric_row(
    frame: &mut Frame<'_>,
    area: Rect,
    metrics: &[(&str, String, Color)],
    app: &App,
) {
    if area.is_empty() || metrics.is_empty() {
        return;
    }
    let cells = Layout::horizontal(vec![
        Constraint::Ratio(1, metrics.len() as u32);
        metrics.len()
    ])
    .spacing(1)
    .split(area);
    for (cell, (label, value, color)) in cells.iter().zip(metrics) {
        let label_width = cell.width.min(5);
        let [label_area, value_area] =
            Layout::horizontal([Constraint::Length(label_width), Constraint::Fill(1)]).areas(*cell);
        frame.render_widget(
            Paragraph::new(format!(" {label}"))
                .style(accent(app, *color).add_modifier(Modifier::BOLD)),
            label_area,
        );
        frame.render_widget(
            Paragraph::new(value.as_str()).alignment(Alignment::Right),
            value_area,
        );
    }
}

fn render_labeled_graph(
    frame: &mut Frame<'_>,
    area: Rect,
    label: &str,
    current: String,
    data: Vec<u64>,
    color: Color,
    app: &App,
) {
    if area.is_empty() {
        return;
    }
    let render_heading = |frame: &mut Frame<'_>, area: Rect| {
        let label_width = area.width.min(label.chars().count() as u16 + 2);
        let [label_area, value_area] =
            Layout::horizontal([Constraint::Length(label_width), Constraint::Fill(1)]).areas(area);
        frame.render_widget(
            Paragraph::new(format!(" {label}"))
                .style(accent(app, color).add_modifier(Modifier::BOLD)),
            label_area,
        );
        frame.render_widget(
            Paragraph::new(current.as_str()).alignment(Alignment::Right),
            value_area,
        );
    };
    if area.height == 1 {
        render_heading(frame, area);
        return;
    }
    let [label_area, graph] =
        Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(area);
    render_heading(frame, label_area);
    render_sparkline(frame, graph, data, color, app);
}

fn wrap_tokens(value: &str, width: usize, max_lines: usize) -> Vec<String> {
    if width == 0 || max_lines == 0 {
        return Vec::new();
    }
    let words = value.split_whitespace().collect::<Vec<_>>();
    if words.is_empty() {
        return vec!["—".to_owned()];
    }
    let mut lines = Vec::with_capacity(max_lines);
    let mut current = String::new();
    for (index, word) in words.iter().enumerate() {
        let next_width =
            current.chars().count() + usize::from(!current.is_empty()) + word.chars().count();
        if next_width <= width {
            if !current.is_empty() {
                current.push(' ');
            }
            current.push_str(word);
            continue;
        }
        if lines.len() + 1 == max_lines {
            if !current.is_empty() {
                current.push(' ');
            }
            current.push_str(&words[index..].join(" "));
            lines.push(format::truncate(&current, width));
            return lines;
        }
        if current.is_empty() {
            lines.push(format::truncate(word, width));
        } else {
            lines.push(current);
            current = (*word).to_owned();
        }
    }
    if !current.is_empty() && lines.len() < max_lines {
        lines.push(format::truncate(&current, width));
    }
    lines
}
