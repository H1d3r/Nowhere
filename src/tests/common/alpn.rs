// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

//! Tests that the TLS Mux marker cannot overlap dedicated flow headers.

use super::*;

#[test]
fn mux_marker_cannot_start_a_dedicated_flow_header() {
    assert!(crate::protocol::decode_flow_header(&[MUX_MARKER, 0, 0, 0, 1]).is_err());
}
