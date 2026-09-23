// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

//! Tests for terminal capability detection.

use super::*;

#[test]
fn capability_detection_is_well_formed() {
    let capabilities = terminal_capabilities();
    let _ = (capabilities.color, capabilities.unicode);
}
