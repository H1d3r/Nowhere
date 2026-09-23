// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

use super::*;

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum WorkspaceDensity {
    Full,
    Compact,
    Narrow,
}

impl WorkspaceDensity {
    pub(super) const fn sidebar_width(self) -> u16 {
        match self {
            Self::Full => 27,
            Self::Compact => 25,
            Self::Narrow => 22,
        }
    }
}

pub(super) fn workspace_columns(area: Rect, density: WorkspaceDensity) -> [Rect; 2] {
    Layout::horizontal([
        Constraint::Length(density.sidebar_width()),
        Constraint::Fill(1),
    ])
    .areas(area)
}

pub(super) fn overview_cards_height(height: u16, density: WorkspaceDensity) -> u16 {
    match density {
        WorkspaceDensity::Full => ((height * 2) / 5).clamp(11, 18),
        WorkspaceDensity::Compact => ((height * 2) / 5).clamp(10, 15),
        WorkspaceDensity::Narrow => height,
    }
}

pub(super) fn log_rows(area: Rect) -> [Rect; 2] {
    Layout::vertical([Constraint::Ratio(1, 2), Constraint::Ratio(1, 2)]).areas(area)
}
