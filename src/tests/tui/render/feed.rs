// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

use super::*;

#[test]
fn access_shows_only_source_and_target() {
    let mut app = app_with_instance();
    show_logs(&mut app);
    app.apply(UiEvent::Access {
        id: "test".to_owned(),
        record: AccessRecord {
            timestamp_ms: 1,
            event_id: 7,
            phase: AccessPhase::Start,
            protocol: "TCP".to_owned(),
            client: Some("10.20.30.40:1234".to_owned()),
            path_peers: vec!["10.20.30.40:1234".to_owned(), "10.20.30.40:5678".to_owned()],
            route: "UP 10.20.30.40:1234 -> relay | DOWN relay -> 10.20.30.40:5678".to_owned(),
            target: Some("example:443".to_owned()),
            ..AccessRecord::default()
        },
    });
    let masked = rendered(200, 32, &app);
    assert!(masked.contains("<redacted>"));
    assert!(masked.contains("example:443"));
    assert!(!masked.contains("10.20.30.40:1234"));
    assert!(!masked.contains("10.20.30.40:5678"));
    assert!(!masked.contains("relay"));

    let revealed = rendered(200, 32, &app);
    assert!(!revealed.contains("10.20.30.40:1234"));
    assert!(!revealed.contains("10.20.30.40:5678"));
    assert!(!revealed.contains("relay"));
}

#[test]
fn runtime_peer_cannot_be_revealed() {
    let mut app = app_with_instance();
    app.apply(UiEvent::Runtime {
        id: "test".to_owned(),
        record: RuntimeRecord {
            timestamp_ms: 1,
            level: EventLevel::Warn,
            kind: "AUTHENTICATION".to_owned(),
            message: "handshake failed".to_owned(),
            client: Some("10.20.30.40:1234".to_owned()),
        },
    });
    app.set_feed(FeedKind::Runtime);
    let masked = rendered(120, 32, &app);
    assert!(masked.contains("<redacted>"));
    assert!(!masked.contains("10.20.30.40:1234"));

    assert!(!rendered(120, 32, &app).contains("10.20.30.40:1234"));
}

#[test]
fn runtime_tail_is_available_through_horizontal_scrolling() {
    let mut app = app_with_instance();
    app.apply(UiEvent::Runtime {
        id: "test".to_owned(),
        record: RuntimeRecord {
            timestamp_ms: 1,
            level: EventLevel::Info,
            kind: "CARRIER".to_owned(),
            message:
                "carrier handshake exceeded the usual row width; accepted on second-row-marker"
                    .to_owned(),
            client: None,
        },
    });
    app.set_feed(FeedKind::Runtime);

    assert!(!rendered(72, 20, &app).contains("second-row-marker"));
    app.runtime_horizontal_scroll = 60;
    assert!(rendered(72, 20, &app).contains("second-row-marker"));
}

#[test]
fn access_prioritizes_complete_route_over_optional_stats() {
    let mut app = app_with_instance();
    show_logs(&mut app);
    app.apply(UiEvent::Access {
        id: "test".to_owned(),
        record: AccessRecord {
            timestamp_ms: 1,
            event_id: 8,
            phase: AccessPhase::Finish,
            protocol: "TCP".to_owned(),
            client: Some("10.20.30.40:1234".to_owned()),
            target: Some("destination.example:443".to_owned()),
            status: Some(AccessStatus::Success),
            duration_ms: Some(42_000),
            upload_bytes: Some(1 << 30),
            download_bytes: Some(2 << 30),
            ..AccessRecord::default()
        },
    });

    let output = rendered(200, 32, &app);
    assert!(output.contains("<redacted>"));
    assert!(output.contains("destination.example:443"));
}

#[test]
fn access_shows_short_client_target_and_carrier_pair() {
    let mut app = app_with_instance();
    show_logs(&mut app);
    app.apply(UiEvent::Access {
        id: "test".to_owned(),
        record: AccessRecord {
            timestamp_ms: 1,
            event_id: 8,
            phase: AccessPhase::Finish,
            protocol: "TCP".to_owned(),
            client: Some("C001".to_owned()),
            target: Some("example.com:443".to_owned()),
            route: "TLS → QUIC".to_owned(),
            status: Some(AccessStatus::Error),
            duration_ms: Some(462),
            message: Some("connection refused".to_owned()),
            ..AccessRecord::default()
        },
    });
    let output = rendered(160, 32, &app);
    assert!(output.contains("C001 → example.com:443"));
    assert!(output.contains("TLS → QUIC"));
    assert!(output.contains("connection refused"));
    assert!(!output.contains("<redacted>"));
    app.capabilities.unicode = false;
    let output = rendered(160, 32, &app);
    assert!(output.contains("C001 > example.com:443"));
    assert!(output.contains("TLS > QUIC"));
    app.selected_mut().unwrap().access.back_mut().unwrap().route = "MIX → TLS".to_owned();
    assert!(rendered(160, 32, &app).contains("MIX > TLS"));
}

#[test]
fn access_error_message_stays_on_the_status_row() {
    let mut app = app_with_instance();
    show_logs(&mut app);
    app.apply(UiEvent::Access {
        id: "test".to_owned(),
        record: AccessRecord {
            timestamp_ms: 1,
            event_id: 9,
            phase: AccessPhase::Finish,
            protocol: "TCP".to_owned(),
            client: Some("10.20.30.40:1234".to_owned()),
            target: Some("example:443".to_owned()),
            status: Some(AccessStatus::Error),
            message: Some("dial failed: dedicated-error-row".to_owned()),
            ..AccessRecord::default()
        },
    });

    let output = rendered(240, 32, &app);
    assert!(
        output
            .lines()
            .any(|line| line.contains("ERR") && line.contains("dedicated-error-row"))
    );
}

#[test]
fn runtime_error_message_stays_on_the_event_row() {
    let mut app = app_with_instance();
    show_logs(&mut app);
    app.apply(UiEvent::Runtime {
        id: "test".to_owned(),
        record: RuntimeRecord {
            timestamp_ms: 1,
            level: EventLevel::Error,
            kind: "IPC".to_owned(),
            message: "connection failed: dedicated-runtime-row".to_owned(),
            client: None,
        },
    });

    let output = rendered(240, 32, &app);
    assert!(
        output
            .lines()
            .any(|line| line.contains("IPC") && line.contains("dedicated-runtime-row"))
    );
}
