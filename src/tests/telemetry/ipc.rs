// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

//! Telemetry registry discovery, frame I/O, and client lifecycle tests.

use std::time::Duration;

use tokio::io::AsyncWriteExt;
use tokio_util::sync::CancellationToken;

use super::frame::{FrameReader, TelemetryReader, write_frame, write_payload_with_timeout};
#[cfg(unix)]
use super::registry::registry_directory;
use super::registry::{RegistryEntry, registry_path};
use super::*;
use crate::telemetry::local;
use crate::telemetry::wire::InstanceDescriptor;
use crate::telemetry::{
    ClientMessage, MAX_FRAME_SIZE, ServerMessage, Subscription, TELEMETRY_PROTOCOL, TelemetryHub,
};
use crate::telemetry::{InstanceRole, RuntimeEvent, RuntimeKind, RuntimeLevel, TelemetrySnapshot};

#[test]
fn snapshot_round_trips_transport_counters() {
    let snapshot = TelemetrySnapshot {
        tls_carriers_active: 2,
        quic_carriers_active: 3,
        tcp_logical_up: 4,
        ..TelemetrySnapshot::default()
    };
    let encoded = serde_json::to_vec(&snapshot).unwrap();
    let decoded: TelemetrySnapshot = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(decoded, snapshot);
}

#[tokio::test]
async fn framing_round_trips_and_rejects_oversize_lengths() {
    let (mut left, mut right) = tokio::io::duplex(MAX_FRAME_SIZE + 16);
    let message = ClientMessage::Subscribe {
        request_id: 0,
        subscription: Subscription::Detail,
    };
    write_frame(&mut left, &message).await.unwrap();
    let mut framed = FrameReader::new(&mut right);
    let decoded: ClientMessage = framed.next().await.unwrap();
    assert_eq!(decoded, message);

    let (mut left, mut right) = tokio::io::duplex(16);
    left.write_u32((MAX_FRAME_SIZE + 1) as u32).await.unwrap();
    let mut framed = FrameReader::new(&mut right);
    let result = framed.next::<ClientMessage>().await;
    assert!(result.is_err());
}

#[tokio::test]
async fn frame_reader_survives_cancelled_partial_reads() {
    let message = ClientMessage::Subscribe {
        request_id: 0,
        subscription: Subscription::Detail,
    };
    let payload = serde_json::to_vec(&message).unwrap();
    let length = (payload.len() as u32).to_be_bytes();
    let (mut writer, reader) = tokio::io::duplex(MAX_FRAME_SIZE + 16);
    let mut framed = FrameReader::new(reader);

    writer.write_all(&length[..2]).await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(10), framed.next::<ClientMessage>())
            .await
            .is_err()
    );
    writer.write_all(&length[2..]).await.unwrap();
    writer.write_all(&payload[..1]).await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(10), framed.next::<ClientMessage>())
            .await
            .is_err()
    );
    writer.write_all(&payload[1..]).await.unwrap();

    assert_eq!(framed.next::<ClientMessage>().await.unwrap(), message);
}

#[tokio::test]
async fn slow_frame_writes_time_out() {
    let (mut writer, _reader) = tokio::io::duplex(1);
    let payload = vec![b'x'; 1_024];
    let result = write_payload_with_timeout(&mut writer, &payload, Duration::from_millis(10)).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("timed out"));
}

async fn next_kind(reader: &mut TelemetryReader, kind: &str) -> ServerMessage {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let message = reader.next_message().await.unwrap();
            if serde_json::to_value(&message).unwrap()["type"] == kind {
                return message;
            }
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn multiple_clients_can_read_and_change_subscriptions() {
    let descriptor = InstanceDescriptor::current(
        InstanceRole::Portal,
        ":2000",
        "secret",
        Duration::from_secs(1),
    )
    .unwrap();
    let discovered = DiscoveredInstance {
        registry_name: descriptor.registry_name(),
        uid: descriptor.uid,
        pid: descriptor.pid,
        incarnation: descriptor.incarnation,
    };
    let hub = TelemetryHub::new(descriptor);
    let shutdown = CancellationToken::new();
    let server = TelemetryServer::bind(hub.clone()).unwrap();
    let path = server.registry_path.clone();
    let _endpoint = server.endpoint.clone();
    let task = tokio::spawn(server.run(shutdown.clone()));
    let summary = TelemetryClient::connect(&discovered, Subscription::Summary)
        .await
        .unwrap();
    let detail = TelemetryClient::connect(&discovered, Subscription::Detail)
        .await
        .unwrap();
    let (_, mut sr, mut sw) = summary.into_parts();
    let (_, mut dr, _) = detail.into_parts();
    next_kind(&mut sr, "subscribed").await;
    next_kind(&mut dr, "subscribed").await;
    next_kind(&mut sr, "snapshot").await;
    next_kind(&mut sr, "lifecycle").await;
    next_kind(&mut dr, "snapshot").await;
    next_kind(&mut dr, "lifecycle").await;
    hub.emit_runtime(RuntimeEvent::new(
        RuntimeLevel::Warn,
        RuntimeKind::Listener,
        "secret.example:443",
    ));
    let event = next_kind(&mut dr, "runtime_event").await;
    assert!(!serde_json::to_string(&event).unwrap().contains("secret"));
    assert!(
        tokio::time::timeout(Duration::from_millis(30), sr.next_message())
            .await
            .is_err()
    );
    sw.subscribe(Subscription::Detail).await.unwrap();
    next_kind(&mut sr, "subscribed").await;
    hub.emit_runtime(RuntimeEvent::new(
        RuntimeLevel::Warn,
        RuntimeKind::Listener,
        "private",
    ));
    next_kind(&mut sr, "runtime_event").await;
    sw.subscribe(Subscription::Summary).await.unwrap();
    next_kind(&mut sr, "subscribed").await;
    shutdown.cancel();
    task.await.unwrap();
    assert!(!path.exists());
    #[cfg(unix)]
    assert!(!std::path::Path::new(&_endpoint).exists());
}

#[tokio::test]
async fn summary_clients_do_not_enable_access_collection() {
    let descriptor = InstanceDescriptor::current(
        InstanceRole::Vector,
        "private",
        "secret",
        Duration::from_secs(1),
    )
    .unwrap();
    let discovered = DiscoveredInstance {
        registry_name: descriptor.registry_name(),
        uid: descriptor.uid,
        pid: descriptor.pid,
        incarnation: descriptor.incarnation,
    };
    let hub = TelemetryHub::new(descriptor);
    let shutdown = CancellationToken::new();
    let server = TelemetryServer::bind(hub.clone()).unwrap();
    let task = tokio::spawn(server.run(shutdown.clone()));
    let client = TelemetryClient::connect(&discovered, Subscription::Summary)
        .await
        .unwrap();
    let (_, mut reader, _) = client.into_parts();
    next_kind(&mut reader, "subscribed").await;
    let _span = hub.start_access(|| panic!("summary must not build access"));
    shutdown.cancel();
    task.await.unwrap();
}

#[tokio::test]
async fn registry_round_trips_and_endpoint_is_local() {
    let descriptor = InstanceDescriptor::current(
        InstanceRole::Portal,
        "secret",
        "secret",
        Duration::from_secs(1),
    )
    .unwrap();
    let server = TelemetryServer::bind(TelemetryHub::new(descriptor)).unwrap();
    let value: serde_json::Value =
        serde_json::from_slice(&local::read_registry(&server.registry_path).unwrap()).unwrap();
    let decoded: RegistryEntry = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(serde_json::to_value(decoded).unwrap(), value);
    assert_eq!(value["protocol"], TELEMETRY_PROTOCOL);
    assert_eq!(value["transport"], local::TRANSPORT);
    assert!(!value.to_string().contains("secret"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&server.registry_path)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(&server.endpoint)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert!(
            local::connect("127.0.0.1:30000", std::process::id())
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn connection_limit_does_not_spawn_extra_clients() {
    let descriptor =
        InstanceDescriptor::current(InstanceRole::Portal, "local", "", Duration::from_secs(1))
            .unwrap();
    let endpoint = local::endpoint(&descriptor.id);
    let server = TelemetryServer::bind(TelemetryHub::new(descriptor)).unwrap();
    let shutdown = CancellationToken::new();
    let task = tokio::spawn(server.run(shutdown.clone()));
    let mut clients = Vec::new();
    for _ in 0..MAX_CLIENTS {
        let stream = local::connect(&endpoint, std::process::id()).await.unwrap();
        let (reader, writer) = tokio::io::split(stream);
        let mut reader = FrameReader::new(reader);
        assert!(matches!(
            reader.next::<ServerMessage>().await.unwrap(),
            ServerMessage::Hello(_)
        ));
        clients.push((reader, writer));
    }
    let stream = local::connect(&endpoint, std::process::id()).await.unwrap();
    let (reader, _) = tokio::io::split(stream);
    let mut reader = FrameReader::new(reader);
    assert!(
        tokio::time::timeout(Duration::from_secs(1), reader.next::<ServerMessage>())
            .await
            .unwrap()
            .is_err()
    );
    shutdown.cancel();
    task.await.unwrap();
}

#[tokio::test]
async fn command_length_and_partial_frame_deadline_are_bounded() {
    let (mut writer, reader) = tokio::io::duplex(2048);
    writer.write_u32(1025).await.unwrap();
    let mut reader = FrameReader::new(reader);
    assert!(reader.next_command().await.is_err());
    let (mut writer, reader) = tokio::io::duplex(16);
    writer.write_all(&[0]).await.unwrap();
    let mut reader = FrameReader::new(reader);
    assert!(
        tokio::time::timeout(Duration::from_millis(10), reader.next_command())
            .await
            .is_err()
    );
    reader.started_at = Some(tokio::time::Instant::now() - Duration::from_secs(6));
    assert!(reader.next_command().await.is_err());
}

#[tokio::test]
async fn namespace_mismatch_is_ignored_without_deletion() {
    let descriptor =
        InstanceDescriptor::current(InstanceRole::Portal, "local", "", Duration::from_secs(1))
            .unwrap();
    let server = TelemetryServer::bind(TelemetryHub::new(descriptor)).unwrap();
    let mut value: serde_json::Value =
        serde_json::from_slice(&local::read_registry(&server.registry_path).unwrap()).unwrap();
    value["namespace"] = serde_json::json!("different namespace");
    std::fs::remove_file(&server.registry_path).unwrap();
    local::publish(&server.registry_path, &serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(
        !discover_instances()
            .unwrap()
            .iter()
            .any(|i| i.registry_name == server.hub.descriptor().registry_name())
    );
    assert!(server.registry_path.exists());
    #[cfg(unix)]
    assert!(std::path::Path::new(&server.endpoint).exists());
}

#[cfg(unix)]
#[tokio::test]
async fn symlink_registry_is_rejected_without_following_it() {
    let descriptor =
        InstanceDescriptor::current(InstanceRole::Portal, "local", "", Duration::from_secs(1))
            .unwrap();
    let server = TelemetryServer::bind(TelemetryHub::new(descriptor)).unwrap();
    let link = server.registry_path.with_extension("link");
    std::os::unix::fs::symlink(&server.registry_path, &link).unwrap();
    assert!(local::read_registry(&link).is_err());
    std::fs::remove_file(&link).unwrap();
    assert!(server.registry_path.exists());
    let long_path = registry_directory().join(format!("{}.sock", "a".repeat(200)));
    assert!(local::Listener::bind(long_path.to_str().unwrap()).is_err());
    assert!(!long_path.exists());
}

#[tokio::test]
async fn command_flood_is_disconnected_and_shutdown_reclaims_detail() {
    let descriptor =
        InstanceDescriptor::current(InstanceRole::Portal, "local", "", Duration::from_secs(1))
            .unwrap();
    let discovered = DiscoveredInstance {
        registry_name: descriptor.registry_name(),
        uid: descriptor.uid,
        pid: descriptor.pid,
        incarnation: descriptor.incarnation,
    };
    let hub = TelemetryHub::new(descriptor);
    let server = TelemetryServer::bind(hub.clone()).unwrap();
    let shutdown = CancellationToken::new();
    let task = tokio::spawn(server.run(shutdown.clone()));
    let client = TelemetryClient::connect(&discovered, Subscription::Detail)
        .await
        .unwrap();
    let (_, mut reader, mut writer) = client.into_parts();
    next_kind(&mut reader, "subscribed").await;
    for _ in 0..20 {
        if writer.subscribe(Subscription::Detail).await.is_err() {
            break;
        }
    }
    tokio::time::timeout(Duration::from_secs(2), async {
        while reader.next_message().await.is_ok() {}
    })
    .await
    .unwrap();
    shutdown.cancel();
    task.await.unwrap();
    let _span = hub.start_access(|| panic!("disconnected detail guard leaked"));
}

#[tokio::test]
async fn failed_publication_rolls_back_only_owned_resources() {
    let descriptor =
        InstanceDescriptor::current(InstanceRole::Portal, "local", "", Duration::from_secs(1))
            .unwrap();
    local::prepare_directory().unwrap();
    let path = registry_path(&descriptor.registry_name());
    #[cfg(unix)]
    let endpoint = local::endpoint(&descriptor.id);
    local::publish(&path, b"{}").unwrap();
    assert!(TelemetryServer::bind(TelemetryHub::new(descriptor)).is_err());
    assert_eq!(local::read_registry(&path).unwrap(), b"{}");
    assert!(!path.with_extension("pending").exists());
    #[cfg(unix)]
    assert!(!std::path::Path::new(&endpoint).exists());
    let pending = path.with_extension("pending");
    let seed = path.with_extension("seed");
    local::publish(&seed, b"{}").unwrap();
    std::fs::rename(seed, &pending).unwrap();
    assert!(local::publish(&path, b"new").is_err());
    assert!(pending.exists());
    std::fs::remove_file(pending).unwrap();
    std::fs::remove_file(path).unwrap();
}
