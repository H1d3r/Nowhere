// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

//! Keyboard handling for the read-only TUI.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use super::model::{App, Focus, Page};

/// Applies one key event.  Returns `true` when a redraw is useful.
pub fn handle_key(app: &mut App, key: KeyEvent) -> bool {
    if key.kind == KeyEventKind::Release {
        return false;
    }

    if app.filter_editing {
        return handle_filter_key(app, key);
    }

    if app.show_help {
        match key.code {
            KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q') => {
                app.show_help = false;
                return true;
            }
            _ => return false,
        }
    }

    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        app.should_quit = true;
        return true;
    }

    match key.code {
        KeyCode::Char('q') => app.should_quit = true,
        KeyCode::Char('?') => app.show_help = true,
        KeyCode::Char('/') if app.page == Page::Logs => app.filter_editing = true,
        KeyCode::Char('c') if app.page == Page::Logs => app.clear_current_feed(),
        KeyCode::Char('p') => app.reveal_clients = !app.reveal_clients,
        KeyCode::Char(' ') if app.page == Page::Logs => {
            app.paused = !app.paused;
            if !app.paused {
                app.feed_scroll = 0;
            }
        }
        KeyCode::Char('1') => {
            app.page = Page::Overview;
            app.focus = Focus::Instances;
        }
        KeyCode::Char('2') => {
            app.page = Page::Logs;
            app.focus = Focus::Instances;
        }
        KeyCode::Tab if key.modifiers.contains(KeyModifiers::SHIFT) => {
            app.focus_previous();
        }
        KeyCode::Tab | KeyCode::BackTab => {
            if matches!(key.code, KeyCode::BackTab) {
                app.focus_previous();
            } else {
                app.focus_next();
            }
        }
        KeyCode::Up | KeyCode::Char('k') => match app.focus {
            Focus::Instances => app.select_relative(-1),
            Focus::Feed => app.scroll_feed(1),
        },
        KeyCode::Down | KeyCode::Char('j') => match app.focus {
            Focus::Instances => app.select_relative(1),
            Focus::Feed => app.scroll_feed(-1),
        },
        KeyCode::Left | KeyCode::Char('h') => match app.focus {
            Focus::Instances => app.select_relative(-1),
            Focus::Feed => app.scroll_feed_horizontal(-4),
        },
        KeyCode::Right | KeyCode::Char('l') => match app.focus {
            Focus::Instances => app.select_relative(1),
            Focus::Feed => app.scroll_feed_horizontal(4),
        },
        KeyCode::PageUp if app.page == Page::Logs => {
            app.focus = Focus::Feed;
            app.scroll_feed(10);
        }
        KeyCode::PageDown if app.page == Page::Logs => {
            app.focus = Focus::Feed;
            app.scroll_feed(-10);
        }
        KeyCode::Home => match app.focus {
            Focus::Instances => app.select_first(),
            Focus::Feed => {
                app.feed_scroll = app.filtered_feed_len().saturating_sub(1);
                app.paused = app.feed_scroll != 0;
            }
        },
        KeyCode::End => match app.focus {
            Focus::Instances => app.select_last(),
            Focus::Feed => {
                app.feed_scroll = 0;
                app.paused = false;
            }
        },
        _ => return false,
    }
    true
}

fn handle_filter_key(app: &mut App, key: KeyEvent) -> bool {
    match key.code {
        KeyCode::Esc | KeyCode::Enter => {
            app.filter_editing = false;
            app.feed_scroll = 0;
            app.reset_horizontal_scroll();
        }
        KeyCode::Backspace => {
            app.filter.pop();
            app.feed_scroll = 0;
            app.reset_horizontal_scroll();
        }
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.filter.clear();
            app.feed_scroll = 0;
            app.reset_horizontal_scroll();
        }
        KeyCode::Char(character)
            if !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
        {
            app.filter.push(character);
            app.feed_scroll = 0;
            app.reset_horizontal_scroll();
        }
        _ => return false,
    }
    true
}

#[cfg(test)]
#[path = "../tests/tui/input.rs"]
mod tests;
