// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

//! Lifecycle vocabulary and transition tests.

use super::*;

#[test]
fn lifecycle_vocabulary_is_stable() {
    assert_eq!(LifeState::Starting.to_string(), "STARTING");
    assert_eq!(LifeState::Ready.to_string(), "READY");
    assert_eq!(LifeState::Draining.to_string(), "DRAINING");
    assert_eq!(LifeState::Stopped.to_string(), "STOPPED");

    let reasons = [
        (LifeReason::Startup, "STARTUP"),
        (LifeReason::Listening, "LISTENING"),
        (LifeReason::SigInt, "SIGINT"),
        #[cfg(unix)]
        (LifeReason::SigTerm, "SIGTERM"),
        (LifeReason::TcpListenerExit, "TCP_LISTENER_EXIT"),
        (LifeReason::QuicListenerExit, "QUIC_LISTENER_EXIT"),
        (LifeReason::SocksListenerExit, "SOCKS_LISTENER_EXIT"),
        (LifeReason::Drained, "DRAINED"),
        (LifeReason::CleanupComplete, "CLEANUP_COMPLETE"),
        (LifeReason::Timeout, "TIMEOUT"),
        (LifeReason::Forced, "FORCED"),
        (LifeReason::StartFailed, "START_FAILED"),
    ];
    for (reason, expected) in reasons {
        assert_eq!(reason.to_string(), expected);
    }
}
