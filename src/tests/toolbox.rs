// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

//! Tests for the toolbox utilities.

use std::time::Duration;

use url::Url;

use super::*;

#[test]
fn panel_aligns_labels_and_values() {
    assert_eq!(
        panel(
            "NOWHERE PROBE",
            &[("Result", "OK".to_owned()), ("Setup", "1.2 ms".to_owned()),],
        ),
        "NOWHERE PROBE\n─────────────\nResult  OK\n Setup  1.2 ms"
    );
}

#[test]
fn duration_uses_compact_consistent_precision() {
    assert_eq!(format_duration(Duration::from_micros(1_250)), "1.2 ms");
    assert_eq!(format_duration(Duration::from_millis(25)), "25 ms");
}

#[test]
fn toolbox_probe_url_allows_missing_socks() {
    let url = Url::parse("vector://secret@127.0.0.1:2000?up=tcp&down=tcp").unwrap();
    assert!(ToolboxClient::parse(&url).is_ok());
}

#[test]
fn setup_results_use_cli_labels() {
    assert_eq!(setup_name(SetupResult::DialFailed), "DIAL_FAILED");
}

#[tokio::test]
async fn status_reads_a_real_local_snapshot_and_redacts_metadata() {
    use crate::telemetry::{DiscoveredInstance, TelemetryHub, TelemetryServer};
    use crate::transport::Stats;
    use tokio_util::sync::CancellationToken;

    let hub = TelemetryHub::for_current_process(
        InstanceRole::Portal,
        "secret@localhost:2000",
        "secret",
        Duration::from_secs(1),
    );
    let descriptor = hub.descriptor();
    let discovered = DiscoveredInstance {
        registry_name: descriptor.registry_name(),
        uid: descriptor.uid,
        pid: descriptor.pid,
        incarnation: descriptor.incarnation,
    };
    let server = TelemetryServer::bind(hub.clone()).unwrap();
    let shutdown = CancellationToken::new();
    let task = tokio::spawn(server.run(shutdown.clone()));
    hub.capture_and_publish(&Stats::default(), 0);
    let item = read_status(discovered).await.unwrap();
    assert_eq!(item.id, hub.descriptor().id);
    assert_eq!(item.endpoint, "<redacted>");
    assert_eq!(item.role, "PORTAL");
    shutdown.cancel();
    task.await.unwrap();
}

#[test]
fn status_uses_vertical_rows_without_removed_fields() {
    let item = StatusItem {
        role: "PORTAL",
        pid: 42,
        id: "instance".to_owned(),
        lifecycle: "READY".to_owned(),
        endpoint: "127.0.0.1:2000".to_owned(),
        snapshot: TelemetrySnapshot::default(),
    };
    let output = status_panel(&item);
    assert!(output.contains("PORTAL · PID 42"));
    assert!(!output.contains("CPU"));
    assert!(!output.contains("Memory"));
    assert!(!output.contains("Flows and carriers"));
}
