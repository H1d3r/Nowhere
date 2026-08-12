// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

//! Semantic colors shared by every TUI panel.

use ratatui::style::Color;

pub(super) const UP: Color = Color::LightGreen;
pub(super) const DOWN: Color = Color::LightBlue;
pub(super) const TCP: Color = Color::LightCyan;
pub(super) const UDP: Color = Color::LightYellow;
pub(super) const TLS: Color = Color::LightMagenta;
pub(super) const QUIC: Color = Color::LightRed;

pub(super) const PING: Color = Color::LightGreen;
pub(super) const POOL: Color = Color::LightBlue;
pub(super) const CPU: Color = Color::LightYellow;
pub(super) const RSS: Color = Color::LightMagenta;

pub(super) const INFO: Color = Color::LightCyan;
pub(super) const SUCCESS: Color = Color::LightGreen;
pub(super) const WARNING: Color = Color::LightYellow;
pub(super) const FAILURE: Color = Color::LightRed;

pub(super) fn protocol(name: &str) -> Color {
    if name.eq_ignore_ascii_case("tcp") {
        TCP
    } else if name.eq_ignore_ascii_case("udp") {
        UDP
    } else if name.eq_ignore_ascii_case("tls") {
        TLS
    } else if name.eq_ignore_ascii_case("quic") {
        QUIC
    } else {
        INFO
    }
}
