// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

//! Downsampling and compact btop-style time-series graphs.

use std::collections::VecDeque;

use ratatui::prelude::*;
use ratatui::symbols::Marker;
use ratatui::widgets::{Axis, Chart, Dataset, GraphType, Paragraph};

use crate::tui::format;
use crate::tui::model::{App, HistoryPoint};

use super::{accent, dim, palette, panel, render_empty};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum GraphStyle {
    Filled,
    Hollow,
}

pub(super) const TRAFFIC_GRAPH_STYLE: GraphStyle = GraphStyle::Filled;
pub(super) const CONNECTION_GRAPH_STYLE: GraphStyle = GraphStyle::Hollow;
pub(super) const PROCESS_GRAPH_STYLE: GraphStyle = GraphStyle::Hollow;

pub(super) const TRAFFIC_COLORS: [Color; 6] = [
    palette::UP,
    palette::DOWN,
    palette::TCP,
    palette::UDP,
    palette::TLS,
    palette::QUIC,
];

pub(super) fn render_traffic(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let Some(instance) = app.selected() else {
        render_empty(
            frame,
            area,
            " TRAFFIC ",
            "Waiting for a running instance",
            app,
        );
        return;
    };
    let block = panel(" TRAFFIC · ALL SERIES ", false, app);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if instance.history.is_empty() {
        frame.render_widget(
            Paragraph::new("Waiting for the second telemetry sample…")
                .alignment(Alignment::Center)
                .style(dim(app)),
            inner,
        );
        return;
    }

    let wide = inner.width >= 84;
    let capacity = usize::from(if wide {
        inner.width / 3
    } else {
        inner.width / 2
    })
    .max(1);
    let latest = instance.latest_history();
    let series = [
        TrafficSeries::new(
            "UP",
            latest.upload_bps,
            rate_series(&instance.history, capacity, |point| point.upload_bps),
            TRAFFIC_COLORS[0],
            0,
        ),
        TrafficSeries::new(
            "DOWN",
            latest.download_bps,
            rate_series(&instance.history, capacity, |point| point.download_bps),
            TRAFFIC_COLORS[1],
            0,
        ),
        TrafficSeries::new(
            "TCP",
            latest.tcp_bps,
            rate_series(&instance.history, capacity, |point| point.tcp_bps),
            TRAFFIC_COLORS[2],
            1,
        ),
        TrafficSeries::new(
            "UDP",
            latest.udp_bps,
            rate_series(&instance.history, capacity, |point| point.udp_bps),
            TRAFFIC_COLORS[3],
            1,
        ),
        TrafficSeries::new(
            "TLS",
            latest.tls_bps,
            rate_series(&instance.history, capacity, |point| point.tls_bps),
            TRAFFIC_COLORS[4],
            2,
        ),
        TrafficSeries::new(
            "QUIC",
            latest.quic_bps,
            rate_series(&instance.history, capacity, |point| point.quic_bps),
            TRAFFIC_COLORS[5],
            2,
        ),
    ];
    let maxima: [u64; 3] = std::array::from_fn(|group| {
        series
            .iter()
            .filter(|item| item.scale_group == group)
            .flat_map(|item| item.data.iter())
            .copied()
            .max()
            .unwrap_or(1)
            .max(1)
    });
    let cells = traffic_cells(inner, wide);
    let order = if wide {
        [0, 2, 4, 1, 3, 5]
    } else {
        [0, 1, 2, 3, 4, 5]
    };
    for (cell, index) in cells.into_iter().zip(order) {
        let item = &series[index];
        render_rate_graph(
            frame,
            cell,
            RateGraph {
                label: item.label,
                current: item.current,
                data: &item.data,
                maximum: maxima[item.scale_group],
                color: item.color,
            },
            app,
        );
    }
}

struct TrafficSeries {
    label: &'static str,
    current: f64,
    data: Vec<u64>,
    color: Color,
    scale_group: usize,
}

impl TrafficSeries {
    const fn new(
        label: &'static str,
        current: f64,
        data: Vec<u64>,
        color: Color,
        scale_group: usize,
    ) -> Self {
        Self {
            label,
            current,
            data,
            color,
            scale_group,
        }
    }
}

pub(super) fn rate_series(
    history: &VecDeque<HistoryPoint>,
    capacity: usize,
    value: fn(HistoryPoint) -> f64,
) -> Vec<u64> {
    let len = history.len();
    let bins = capacity.min(len).max(1);
    (0..bins)
        .map(|bin| {
            let start = bin * len / bins;
            let end = ((bin + 1) * len / bins).max(start + 1);
            history
                .iter()
                .skip(start)
                .take(end.saturating_sub(start))
                .copied()
                .map(value)
                .fold(0.0_f64, f64::max)
        })
        .map(graph_value)
        .collect()
}

struct RateGraph<'a> {
    label: &'a str,
    current: f64,
    data: &'a [u64],
    maximum: u64,
    color: Color,
}

fn render_rate_graph(frame: &mut Frame<'_>, area: Rect, series: RateGraph<'_>, app: &App) {
    if area.is_empty() {
        return;
    }
    let label_line = Line::from(vec![
        Span::styled(
            format!(" {:<5}", series.label),
            accent(app, series.color).add_modifier(Modifier::BOLD),
        ),
        Span::styled(format::bits_per_second(series.current), Style::default()),
    ]);
    if area.height == 1 {
        frame.render_widget(Paragraph::new(label_line), area);
        return;
    }
    let [heading, graph] =
        Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(area);
    frame.render_widget(Paragraph::new(label_line), heading);
    render_sparkline_with_max(
        frame,
        graph,
        series.data,
        series.maximum,
        series.color,
        TRAFFIC_GRAPH_STYLE,
        app,
    );
}

pub(super) fn traffic_cells(area: Rect, wide: bool) -> Vec<Rect> {
    if wide {
        let [top, bottom] =
            Layout::vertical([Constraint::Ratio(1, 2), Constraint::Ratio(1, 2)]).areas(area);
        let [a, b, c] = Layout::horizontal([
            Constraint::Ratio(1, 3),
            Constraint::Ratio(1, 3),
            Constraint::Ratio(1, 3),
        ])
        .spacing(1)
        .areas(top);
        let [d, e, f] = Layout::horizontal([
            Constraint::Ratio(1, 3),
            Constraint::Ratio(1, 3),
            Constraint::Ratio(1, 3),
        ])
        .spacing(1)
        .areas(bottom);
        vec![a, b, c, d, e, f]
    } else {
        let [top, middle, bottom] = Layout::vertical([
            Constraint::Ratio(1, 3),
            Constraint::Ratio(1, 3),
            Constraint::Ratio(1, 3),
        ])
        .areas(area);
        [top, middle, bottom]
            .into_iter()
            .flat_map(|row| {
                let [left, right] =
                    Layout::horizontal([Constraint::Ratio(1, 2), Constraint::Ratio(1, 2)])
                        .spacing(1)
                        .areas(row);
                [left, right]
            })
            .collect()
    }
}

fn graph_value(value: f64) -> u64 {
    if !value.is_finite() || value <= 0.0 {
        0
    } else {
        value.min(u64::MAX as f64) as u64
    }
}

pub(super) fn scaled_data(
    history: &VecDeque<HistoryPoint>,
    width: u16,
    value: fn(&HistoryPoint) -> f64,
) -> Vec<u64> {
    downsample(history, width, |point| graph_value(value(point) * 10.0))
}

pub(super) fn spark_data(
    history: &VecDeque<HistoryPoint>,
    width: u16,
    value: fn(&HistoryPoint) -> u64,
) -> Vec<u64> {
    downsample(history, width, value)
}

fn downsample(
    history: &VecDeque<HistoryPoint>,
    width: u16,
    value: impl Fn(&HistoryPoint) -> u64,
) -> Vec<u64> {
    let bins = usize::from(width).min(history.len());
    if bins == 0 {
        return Vec::new();
    }
    (0..bins)
        .map(|bin| {
            let start = bin * history.len() / bins;
            let end = ((bin + 1) * history.len() / bins).max(start + 1);
            history
                .iter()
                .skip(start)
                .take(end.saturating_sub(start))
                .map(&value)
                .max()
                .unwrap_or_default()
        })
        .collect()
}

pub(super) fn render_sparkline(
    frame: &mut Frame<'_>,
    area: Rect,
    data: Vec<u64>,
    color: Color,
    app: &App,
) {
    let maximum = data.iter().copied().max().unwrap_or(1).max(1);
    render_sparkline_with_max(frame, area, &data, maximum, color, PROCESS_GRAPH_STYLE, app);
}

pub(super) fn render_dual_sparkline(
    frame: &mut Frame<'_>,
    area: Rect,
    first: Vec<u64>,
    first_color: Color,
    second: Vec<u64>,
    second_color: Color,
    app: &App,
) {
    if area.is_empty() || (first.is_empty() && second.is_empty()) {
        return;
    }
    let maximum = first
        .iter()
        .chain(&second)
        .copied()
        .max()
        .unwrap_or(1)
        .max(1);
    if !app.capabilities.unicode {
        let [first_area, second_area] =
            Layout::vertical([Constraint::Ratio(1, 2), Constraint::Ratio(1, 2)]).areas(area);
        render_sparkline_with_max(
            frame,
            first_area,
            &first,
            maximum,
            first_color,
            CONNECTION_GRAPH_STYLE,
            app,
        );
        render_sparkline_with_max(
            frame,
            second_area,
            &second,
            maximum,
            second_color,
            CONNECTION_GRAPH_STYLE,
            app,
        );
        return;
    }

    let data_width = first.len().max(second.len());
    let visible_width = area
        .width
        .min(u16::try_from(data_width).unwrap_or(u16::MAX));
    if visible_width == 0 {
        return;
    }
    let graph_area = Rect::new(
        area.x + area.width.saturating_sub(visible_width),
        area.y,
        visible_width,
        area.height,
    );
    let points = |data: &[u64]| {
        let visible = &data[data.len().saturating_sub(usize::from(visible_width))..];
        let x_offset = usize::from(visible_width).saturating_sub(visible.len());
        let mut points = visible
            .iter()
            .enumerate()
            .map(|(index, value)| ((x_offset + index) as f64, *value as f64))
            .collect::<Vec<_>>();
        if points.len() == 1 {
            points.push(((x_offset + 1) as f64, points[0].1));
        }
        points
    };
    let first_points = points(&first);
    let second_points = points(&second);
    let mut datasets = Vec::with_capacity(2);
    if !first_points.is_empty() {
        datasets.push(
            Dataset::default()
                .marker(Marker::Braille)
                .graph_type(graph_type(CONNECTION_GRAPH_STYLE))
                .style(accent(app, first_color))
                .data(&first_points),
        );
    }
    if !second_points.is_empty() {
        datasets.push(
            Dataset::default()
                .marker(Marker::Braille)
                .graph_type(graph_type(CONNECTION_GRAPH_STYLE))
                .style(accent(app, second_color).add_modifier(Modifier::BOLD))
                .data(&second_points),
        );
    }
    frame.render_widget(
        Chart::new(datasets)
            .x_axis(
                Axis::default().bounds([0.0, f64::from(visible_width.saturating_sub(1).max(1))]),
            )
            .y_axis(Axis::default().bounds([0.0, maximum as f64])),
        graph_area,
    );
}

fn render_sparkline_with_max(
    frame: &mut Frame<'_>,
    area: Rect,
    data: &[u64],
    maximum: u64,
    color: Color,
    style: GraphStyle,
    app: &App,
) {
    if area.is_empty() || data.is_empty() {
        return;
    }
    let visible_width = area
        .width
        .min(u16::try_from(data.len()).unwrap_or(u16::MAX));
    let graph_area = Rect::new(
        area.x + area.width.saturating_sub(visible_width),
        area.y,
        visible_width,
        area.height,
    );
    let visible = &data[data.len().saturating_sub(usize::from(visible_width))..];
    if app.capabilities.unicode {
        let mut points = visible
            .iter()
            .enumerate()
            .map(|(index, value)| (index as f64, *value as f64))
            .collect::<Vec<_>>();
        if points.len() == 1 {
            points.push((1.0, points[0].1));
        }
        let x_max = points.len().saturating_sub(1).max(1) as f64;
        let dataset = Dataset::default()
            .marker(Marker::Braille)
            .graph_type(graph_type(style))
            .style(accent(app, color))
            .data(&points);
        let dataset = if style == GraphStyle::Filled {
            dataset.fill_to_y(0.0)
        } else {
            dataset
        };
        frame.render_widget(
            Chart::new(vec![dataset])
                .x_axis(Axis::default().bounds([0.0, x_max]))
                .y_axis(Axis::default().bounds([0.0, maximum.max(1) as f64])),
            graph_area,
        );
    } else {
        let symbols = [' ', '.', ':', '*', '#'];
        let line = visible
            .iter()
            .map(|value| {
                let level = value.saturating_mul((symbols.len() - 1) as u64) / maximum.max(1);
                symbols[level as usize]
            })
            .collect::<String>();
        let line_area = Rect::new(
            graph_area.x,
            area.y + area.height.saturating_sub(1),
            visible_width,
            1,
        );
        frame.render_widget(Paragraph::new(line).style(accent(app, color)), line_area);
    }
}

const fn graph_type(style: GraphStyle) -> GraphType {
    match style {
        GraphStyle::Filled => GraphType::Area,
        GraphStyle::Hollow => GraphType::Line,
    }
}
