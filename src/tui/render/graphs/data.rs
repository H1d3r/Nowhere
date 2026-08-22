// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

//! Width-bounded chart sampling that preserves per-bin peaks.

use std::collections::VecDeque;

use crate::tui::model::HistoryPoint;

pub(super) fn graph_value(value: f64) -> u64 {
    if !value.is_finite() || value <= 0.0 {
        0
    } else {
        value.min(u64::MAX as f64) as u64
    }
}

pub(in crate::tui::render) fn scaled_data(
    history: &VecDeque<HistoryPoint>,
    width: u16,
    value: fn(&HistoryPoint) -> f64,
) -> Vec<u64> {
    downsample(history, width, |point| graph_value(value(point) * 10.0))
}

pub(in crate::tui::render) fn spark_data(
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
