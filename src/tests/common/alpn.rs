// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

use super::*;

#[test]
fn alpn_defaults_to_now_one_and_preserves_custom_values() {
    assert_eq!(parse_alpn(None).unwrap(), DEFAULT_ALPN);
    assert_eq!(parse_alpn(Some("private/7")).unwrap(), "private/7");
}

#[test]
fn alpn_rejects_empty_and_oversized_values() {
    assert!(parse_alpn(Some("")).is_err());
    assert!(parse_alpn(Some(&"a".repeat(256))).is_err());
}

#[test]
fn mux_marker_cannot_start_a_dedicated_flow_header() {
    assert!(crate::protocol::decode_flow_header(&[MUX_MARKER, 0, 0, 0, 1]).is_err());
}
