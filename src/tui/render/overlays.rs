// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

//! Help and configuration overlays for the terminal dashboard.

use super::*;
use ratatui::widgets::Clear;

pub(super) fn render_help(frame: &mut Frame<'_>, area: Rect, app: &App) {
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
        Line::from("  i             complete instance configuration"),
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

pub(super) fn render_config(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let Some(instance) = app.selected() else {
        return;
    };
    let popup = centered(
        area,
        area.width.saturating_sub(4).min(100),
        area.height.saturating_sub(4),
    );
    frame.render_widget(Clear, popup);
    let option_count = instance.meta.config_summary.split_whitespace().count() + 1;
    let block = panel(" CONFIG · ↑↓ scroll · Esc/i close ", true, app).title_bottom(
        Line::from(format!(
            " option {}/{} · PgUp/PgDn · Home/End ",
            app.config_scroll.min(option_count - 1) + 1,
            option_count
        ))
        .style(dim(app)),
    );
    let inner = block.inner(popup);
    frame.render_widget(block, popup);
    let mut options = vec![("endpoint", instance.meta.endpoint.as_str())];
    options.extend(
        instance
            .meta
            .config_summary
            .split_whitespace()
            .filter_map(|option| option.split_once('=')),
    );
    let key_width = options
        .iter()
        .map(|(key, _)| key.len())
        .max()
        .unwrap_or(8)
        .min(16);
    let value_width = usize::from(inner.width)
        .saturating_sub(key_width + 3)
        .max(1);
    let mut lines = vec![];
    for (key, value) in options
        .iter()
        .skip(app.config_scroll.min(options.len().saturating_sub(1)))
    {
        let chars: Vec<_> = value.chars().collect();
        for (index, chunk) in chars.chunks(value_width).enumerate() {
            lines.push(Line::from(vec![
                Span::styled(
                    format!(" {:key_width$}  ", if index == 0 { key } else { &"" }),
                    dim(app),
                ),
                Span::raw(chunk.iter().collect::<String>()),
            ]));
        }
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    Rect::new(
        area.x + area.width.saturating_sub(width) / 2,
        area.y + area.height.saturating_sub(height) / 2,
        width.min(area.width),
        height.min(area.height),
    )
}
