// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

use super::*;

pub(super) struct MetricGraph {
    label: &'static str,
    current: String,
    data: Vec<u64>,
    color: Color,
}

impl MetricGraph {
    pub(super) const fn new(
        label: &'static str,
        current: String,
        data: Vec<u64>,
        color: Color,
    ) -> Self {
        Self {
            label,
            current,
            data,
            color,
        }
    }
}

pub(super) fn render_graph_pair(
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

pub(super) fn render_metric_row(
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
