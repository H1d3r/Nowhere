use std::sync::atomic::Ordering;

use crate::protocol::Carrier;
use crate::telemetry::{
    AccessOutcome, AccessStart, InstanceDescriptor, InstanceRole, PROTOCOL_VERSION, ServerMessage,
    TelemetryHub, TrafficProtocol,
};
use crate::transport::Stats;

fn descriptor() -> InstanceDescriptor {
    InstanceDescriptor {
        protocol_version: PROTOCOL_VERSION,
        id: "1:2:3".to_owned(),
        role: InstanceRole::Portal,
        pid: 2,
        uid: 1,
        start_ticks: 3,
        version: "test".to_owned(),
        endpoint: ":2077".to_owned(),
        config_summary: "portal net=mix".to_owned(),
        telemetry_interval_ms: 1_000,
    }
}

#[test]
fn access_span_finishes_only_once() {
    let hub = TelemetryHub::new(descriptor());
    let mut events = hub.event_receiver();
    let span = hub.start_access(AccessStart {
        id: 0,
        timestamp_ms: 1,
        protocol: TrafficProtocol::Tcp,
        flow_id: Some(7),
        client: Some("127.0.0.1:1".to_owned()),
        path_peers: vec!["127.0.0.1:1".to_owned()],
        target: "example:443".to_owned(),
        uplink: Some(Carrier::TlsTcp),
        downlink: Some(Carrier::Quic),
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
fn snapshot_contains_existing_transport_counters() {
    let hub = TelemetryHub::new(descriptor());
    let stats = Stats::default();
    stats.tcp_rx.store(42, Ordering::Relaxed);
    stats.link_tcp.store(2, Ordering::Relaxed);
    hub.capture_and_publish(&stats, 3, 17);
    let snapshot = hub.snapshots.borrow().clone();
    assert_eq!(snapshot.tcp_rx, 42);
    assert_eq!(snapshot.link_tcp, 2);
    assert_eq!(snapshot.pool_active, 3);
    assert_eq!(snapshot.ping_ms, 17);
}
