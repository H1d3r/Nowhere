// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

//! Telemetry wire messages normalized into TUI model events.

use super::*;

pub(super) fn hello_ui_event(hello: &Hello) -> UiEvent {
    let descriptor = &hello.instance;
    UiEvent::Upsert {
        meta: InstanceMeta {
            id: descriptor.id.clone(),
            role: match descriptor.role {
                WireRole::Portal => InstanceRole::Portal,
                WireRole::Vector => InstanceRole::Vector,
            },
            pid: descriptor.pid,
            uid: descriptor.uid,
            version: descriptor.version.clone(),
            endpoint: if descriptor.endpoint.is_empty() {
                "—".to_owned()
            } else {
                crate::telemetry::display_endpoint(&descriptor.endpoint)
            },
            config_summary: crate::telemetry::display_config(&descriptor.config_summary),
            telemetry_interval_ms: descriptor.telemetry_interval_ms,
            telemetry_protocol: descriptor.telemetry_protocol.clone(),
        },
        lifecycle: Lifecycle::from_label(&hello.lifecycle),
        snapshot: None,
    }
}

pub(super) fn server_ui_events(
    message: ServerMessage,
    id: &str,
    starts: &mut HashMap<u64, AccessRecord>,
) -> Vec<UiEvent> {
    match message {
        ServerMessage::Subscribed { .. } => vec![],
        ServerMessage::Hello(hello) => vec![hello_ui_event(&hello)],
        ServerMessage::Snapshot(snapshot) => vec![UiEvent::Snapshot {
            id: id.to_owned(),
            snapshot: snapshot_ui_value(snapshot),
        }],
        ServerMessage::Lifecycle(lifecycle) => vec![UiEvent::Lifecycle {
            id: id.to_owned(),
            lifecycle: Lifecycle::from_label(&lifecycle.state),
        }],
        ServerMessage::RuntimeEvent(event) => {
            vec![UiEvent::Runtime {
                id: id.to_owned(),
                record: runtime_ui_value(event),
            }]
        }
        ServerMessage::AccessStart(start) => {
            let record = access_start_ui_value(start);
            // Finishes are self-contained; bound correlation memory even when
            // active flows are long-lived or their finishes were not delivered.
            if starts.len() >= 2_048 {
                starts.clear();
            }
            starts.insert(record.event_id, record.clone());
            vec![UiEvent::Access {
                id: id.to_owned(),
                record,
            }]
        }
        ServerMessage::AccessFinish(finish) => {
            let record = access_finish_ui_value(finish, starts);
            vec![UiEvent::Access {
                id: id.to_owned(),
                record,
            }]
        }
        ServerMessage::Gap { missed } => {
            starts.clear();
            vec![UiEvent::Gap {
                id: id.to_owned(),
                missed,
            }]
        }
        ServerMessage::Error { message } => vec![UiEvent::Error {
            id: Some(id.to_owned()),
            message,
        }],
    }
}

fn snapshot_ui_value(value: WireSnapshot) -> TelemetrySnapshot {
    TelemetrySnapshot {
        sequence: value.sequence,
        timestamp_ms: value.timestamp_ms,
        uptime_ms: value.uptime_ms,
        tcp_logical_up: value.tcp_logical_up,
        tcp_logical_down: value.tcp_logical_down,
        udp_logical_up: value.udp_logical_up,
        udp_logical_down: value.udp_logical_down,
        tls_payload_up: value.tls_payload_up,
        tls_payload_down: value.tls_payload_down,
        quic_payload_up: value.quic_payload_up,
        quic_payload_down: value.quic_payload_down,
        tcp_active: value.tcp_active,
        udp_active: value.udp_active,
        tls_carriers_active: value.tls_carriers_active,
        quic_carriers_active: value.quic_carriers_active,
        cpu_percent: value.cpu_percent,
        rss_bytes: value.rss_bytes,
    }
}

fn runtime_ui_value(value: RuntimeEvent) -> RuntimeRecord {
    RuntimeRecord {
        timestamp_ms: value.timestamp_ms,
        level: match value.level {
            RuntimeLevel::Info => EventLevel::Info,
            RuntimeLevel::Warn => EventLevel::Warn,
            RuntimeLevel::Error => EventLevel::Error,
        },
        kind: format!("{:?}", value.kind).to_ascii_uppercase(),
        message: value.message,
        client: value.client,
    }
}

pub(super) fn access_start_ui_value(value: AccessStarted) -> AccessRecord {
    let route = value.path.unwrap_or_else(|| {
        let carrier = |value: Option<&str>| match value {
            Some("tcp") => "TLS",
            Some("udp") => "QUIC",
            None => "MIX",
            _ => "?",
        };
        let up = carrier(value.initial_uplink.as_deref());
        let down = carrier(value.initial_downlink.as_deref());
        format!("{up} → {down}")
    });
    AccessRecord {
        timestamp_ms: value.timestamp_ms,
        event_id: value.id,
        phase: AccessPhase::Start,
        protocol: match value.protocol {
            TrafficProtocol::Tcp => "TCP",
            TrafficProtocol::Udp => "UDP",
        }
        .to_owned(),
        session_tag: value.session_tag,
        client: value.client,
        path_peers: value.path_peers,
        route,
        target: Some(value.target),
        ..AccessRecord::default()
    }
}

pub(super) fn access_finish_ui_value(
    value: AccessFinished,
    starts: &mut HashMap<u64, AccessRecord>,
) -> AccessRecord {
    let AccessFinished {
        id,
        timestamp_ms,
        duration_ms,
        protocol,
        flow_id,
        client,
        path_peers,
        target,
        session_tag,
        initial_uplink,
        initial_downlink,
        path,
        upload_bytes,
        download_bytes,
        outcome,
        error,
        ..
    } = value;
    let mut record = starts.remove(&id).unwrap_or_else(|| {
        access_start_ui_value(AccessStarted {
            sequence: 0,
            truncated: false,
            id,
            timestamp_ms,
            protocol,
            flow_id,
            session_tag,
            client,
            path_peers,
            target,
            initial_uplink,
            initial_downlink,
            path,
        })
    });
    record.timestamp_ms = timestamp_ms;
    record.event_id = id;
    record.phase = AccessPhase::Finish;
    let benign_end = error.as_deref().is_some_and(is_benign_access_end);
    record.status = Some(match outcome {
        AccessOutcome::Success => AccessStatus::Success,
        AccessOutcome::Cancelled => AccessStatus::Ended,
        AccessOutcome::Error if benign_end => AccessStatus::Ended,
        AccessOutcome::Error => AccessStatus::Error,
        AccessOutcome::Timeout => AccessStatus::Timeout,
        AccessOutcome::Rejected => AccessStatus::Rejected,
    });
    record.message = match outcome {
        AccessOutcome::Success | AccessOutcome::Cancelled => None,
        AccessOutcome::Error if benign_end => None,
        AccessOutcome::Timeout if error.as_deref() == Some("idle timeout") => None,
        _ => error,
    };
    record.duration_ms = Some(duration_ms);
    record.upload_bytes = Some(upload_bytes);
    record.download_bytes = Some(download_bytes);
    record
}

pub(super) fn is_benign_access_end(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    [
        "broken pipe",
        "connection reset by peer",
        "connection aborted",
        "connection closed",
        "closed by peer",
        "application closed",
        "unexpected eof",
        "early eof",
        "error 256",
        "error code 256",
    ]
    .into_iter()
    .any(|pattern| message.contains(pattern))
}
