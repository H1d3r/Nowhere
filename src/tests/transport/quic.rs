// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

use super::*;

#[test]
fn profiles_keep_streams_within_connection_limits() {
    for profile in [
        parse_quic_profile(Some("memory")).unwrap(),
        parse_quic_profile(Some("balanced")).unwrap(),
        parse_quic_profile(Some("throughput")).unwrap(),
    ] {
        assert!(profile.stream_receive_window <= profile.connection_receive_window);
        assert!(u64::from(profile.connection_receive_window) <= profile.send_window * 2);
    }
}

#[test]
fn balanced_is_the_resource_safe_default() {
    assert_eq!(parse_quic_profile(None).unwrap(), QuicFlowControl::BALANCED);
}

#[test]
fn rejects_unknown_profiles() {
    assert!(parse_quic_profile(Some("tiny")).is_err());
}
