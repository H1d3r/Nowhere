use std::sync::atomic::Ordering;

use crate::protocol::Carrier;
use crate::telemetry::wire::InstanceDescriptor;
use crate::telemetry::{
    AccessOutcome, AccessStart, InstanceRole, RuntimeEvent, RuntimeKind, RuntimeLevel,
    ServerMessage, TELEMETRY_PROTOCOL, TelemetryHub, TrafficProtocol,
};
use crate::transport::Stats;

fn descriptor() -> InstanceDescriptor {
    InstanceDescriptor {
        telemetry_protocol: TELEMETRY_PROTOCOL.to_owned(),
        id: "1:2:3".to_owned(),
        role: InstanceRole::Portal,
        pid: 2,
        uid: 1,
        incarnation: 3,
        version: "test".to_owned(),
        endpoint: ":2000".to_owned(),
        config_summary: "portal net=mix".to_owned(),
        telemetry_interval_ms: 1_000,
    }
}

#[test]
fn descriptor_serializes_operator_metadata_without_local_identity() {
    let hub = TelemetryHub::new(descriptor());
    let encoded = serde_json::to_string(hub.descriptor()).unwrap();
    assert!(encoded.contains("2000"));
    assert!(encoded.contains("config_summary"));
    assert!(!encoded.contains("\"uid\""));
    assert!(!encoded.contains("incarnation"));
}

#[test]
fn access_span_finishes_only_once() {
    let hub = TelemetryHub::new(descriptor());
    let mut events = hub.event_receiver();
    let _detail = hub.detail_guard();
    let span = hub.start_access(|| AccessStart {
        id: 0,
        timestamp_ms: 1,
        protocol: TrafficProtocol::Tcp,
        flow_id: Some(7),
        session_tag: Some("abc123".to_owned()),
        client: Some("127.0.0.1:1".to_owned()),
        path_peers: vec!["127.0.0.1:1".to_owned()],
        target: "example:443".to_owned(),
        initial_uplink: Some(Carrier::TlsTcp),
        initial_downlink: Some(Carrier::Quic),
        path: None,
    });
    span.add_upload(10);
    span.add_download(20);
    span.finish(AccessOutcome::Success, None);

    assert!(matches!(
        events.try_recv(),
        Ok(ServerMessage::AccessStart(_))
    ));
    let Ok(ServerMessage::AccessFinish(finish)) = events.try_recv() else {
        panic!("missing access finish");
    };
    assert_eq!(finish.upload_bytes, 10);
    assert_eq!(finish.download_bytes, 20);
    assert_eq!(finish.outcome, AccessOutcome::Success);
    assert!(events.try_recv().is_err());
}

#[test]
fn access_fields_are_not_built_without_a_receiver() {
    let hub = TelemetryHub::new(descriptor());
    let span = hub.start_access(|| panic!("unobservable access must stay lazy"));

    span.add_upload(10);
    span.add_download(20);
    span.finish(AccessOutcome::Success, None);
}

#[test]
fn snapshot_contains_existing_transport_counters() {
    let hub = TelemetryHub::new(descriptor());
    let stats = Stats::default();
    stats.tcp_rx.store(42, Ordering::Relaxed);
    stats.link_tcp.store(2, Ordering::Relaxed);
    hub.capture_and_publish(&stats, 17);
    let snapshot = hub.snapshots.borrow().clone();
    assert_eq!(snapshot.tcp_logical_up, 42);
    assert_eq!(snapshot.tls_carriers_active, 2);
    assert_eq!(snapshot.ping_ms, 17);
}

#[test]
fn detail_guards_balance_switches_and_drop() {
    let hub = TelemetryHub::new(descriptor());
    let receiver = hub.event_receiver();
    let _span = hub.start_access(|| panic!("a summary receiver is not detail"));
    let first = hub.detail_guard();
    let second = hub.detail_guard();
    assert_eq!(hub.detail_clients.load(Ordering::Relaxed), 2);
    drop(first);
    assert_eq!(hub.detail_clients.load(Ordering::Relaxed), 1);
    drop(second);
    assert_eq!(hub.detail_clients.load(Ordering::Relaxed), 0);
    drop(receiver);
}

#[test]
fn publisher_rechecks_runtime_diagnostics_before_delivery() {
    let hub = TelemetryHub::new(descriptor());
    let mut events = hub.event_receiver();
    let _detail = hub.detail_guard();
    let mut event = RuntimeEvent::new(
        RuntimeLevel::Warn,
        RuntimeKind::Carrier,
        "QUIC carrier connected",
    );
    event.message = "QUIC carrier connected: secret-key /private/key.pem\x1b[31m".to_owned();
    event.client = Some("203.0.113.5:4321".to_owned());
    hub.emit_runtime(event);
    let Ok(ServerMessage::RuntimeEvent(event)) = events.try_recv() else {
        panic!("missing runtime event")
    };
    assert_eq!(event.message, "QUIC carrier connected: operation failed");
    assert_eq!(event.client.as_deref(), Some("C001"));
    let encoded = serde_json::to_string(&event).unwrap();
    assert!(encoded.contains("\"message\""));
    assert!(!encoded.contains("secret-key"));
    assert!(!encoded.contains("203.0.113.5"));
}
