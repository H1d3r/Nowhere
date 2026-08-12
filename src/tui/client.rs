// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

//! Linux IPC-to-view-model adapter.
//!
//! Discovery and wire handling live here so the rest of the TUI depends only
//! on the normalized model in `model.rs`.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use anyhow::Result;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::telemetry::{
    AccessFinished, AccessOutcome, AccessStarted, DiscoveredInstance, Hello,
    InstanceRole as WireRole, RuntimeEvent, RuntimeKind, RuntimeLevel, ServerMessage, Subscription,
    TelemetryClient, TelemetrySnapshot as WireSnapshot, TrafficProtocol, discover_instances,
};

use super::model::{
    AccessPhase, AccessRecord, AccessStatus, EventLevel, InstanceId, InstanceMeta, InstanceRole,
    Lifecycle, RuntimeRecord, TelemetrySnapshot, UiEvent,
};

const DISCOVERY_INTERVAL: Duration = Duration::from_secs(1);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
const CLIENT_EVENT_CAPACITY: usize = 2_048;

/// Control messages from the UI to the telemetry connection manager.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UiCommand {
    /// Subscribe to detail for this instance and summary for all others.
    Select(Option<InstanceId>),
    Shutdown,
}

pub struct ClientHandle {
    pub events: mpsc::Receiver<UiEvent>,
    pub commands: mpsc::UnboundedSender<UiCommand>,
}

/// Starts `/proc/net/unix` discovery and one read-only connection per instance.
pub fn start() -> Result<ClientHandle> {
    let (event_tx, events) = mpsc::channel(CLIENT_EVENT_CAPACITY);
    let (commands, command_rx) = mpsc::unbounded_channel();
    tokio::spawn(connection_manager(event_tx, command_rx));
    Ok(ClientHandle { events, commands })
}

struct Connection {
    id: Option<InstanceId>,
    subscription: Subscription,
    subscription_tx: mpsc::UnboundedSender<Subscription>,
    task: JoinHandle<()>,
    generation: u64,
}

enum ConnectionEvent {
    Identified {
        key: String,
        generation: u64,
        id: InstanceId,
    },
    Ended {
        key: String,
        generation: u64,
        id: Option<InstanceId>,
        error: Option<String>,
    },
}

async fn connection_manager(
    event_tx: mpsc::Sender<UiEvent>,
    mut commands: mpsc::UnboundedReceiver<UiCommand>,
) {
    let (connection_tx, mut connection_rx) = mpsc::unbounded_channel();
    let mut connections: HashMap<String, Connection> = HashMap::new();
    let mut selected: Option<InstanceId> = None;
    let mut generation = 0_u64;
    let mut discovery = tokio::time::interval(DISCOVERY_INTERVAL);
    discovery.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            _ = discovery.tick() => {
                match discover_instances() {
                    Ok(discovered) => {
                        let seen = discovered
                            .iter()
                            .map(|instance| instance.socket_name.clone())
                            .collect::<HashSet<_>>();
                        let stale = connections
                            .keys()
                            .filter(|key| !seen.contains(*key))
                            .cloned()
                            .collect::<Vec<_>>();
                        for key in stale {
                            if let Some(connection) = connections.remove(&key) {
                                connection.task.abort();
                                if let Some(id) = connection.id {
                                    let _ = event_tx.send(UiEvent::Offline { id }).await;
                                }
                            }
                        }
                        for instance in discovered {
                            let key = instance.socket_name.clone();
                            if connections.contains_key(&key) {
                                continue;
                            }
                            generation = generation.wrapping_add(1);
                            connections.insert(
                                key.clone(),
                                spawn_connection(
                                    key,
                                    instance,
                                    generation,
                                    &event_tx,
                                    &connection_tx,
                                ),
                            );
                        }
                    }
                    Err(error) => {
                        let _ = event_tx.send(UiEvent::Error {
                            id: None,
                            message: format!("instance discovery failed: {error}"),
                        }).await;
                    }
                }
            }
            command = commands.recv() => {
                match command {
                    Some(UiCommand::Select(id)) => {
                        selected = id;
                        update_subscriptions(&mut connections, selected.as_deref());
                    }
                    Some(UiCommand::Shutdown) | None => {
                        for (_, connection) in connections {
                            connection.task.abort();
                        }
                        return;
                    }
                }
            }
            event = connection_rx.recv() => {
                let Some(event) = event else {
                    return;
                };
                match event {
                    ConnectionEvent::Identified { key, generation, id } => {
                        if let Some(connection) = connections.get_mut(&key)
                            && connection.generation == generation
                        {
                            connection.id = Some(id);
                            update_subscription(connection, selected.as_deref());
                        }
                    }
                    ConnectionEvent::Ended { key, generation, id, error } => {
                        let current = connections
                            .get(&key)
                            .is_some_and(|connection| connection.generation == generation);
                        if !current {
                            continue;
                        }
                        connections.remove(&key);
                        if let Some(id) = id {
                            if let Some(error) = error {
                                let _ = event_tx.send(UiEvent::Error {
                                    id: Some(id.clone()),
                                    message: error,
                                }).await;
                            }
                            let _ = event_tx.send(UiEvent::Offline { id }).await;
                        } else if let Some(error) = error {
                            let _ = event_tx.send(UiEvent::Error {
                                id: None,
                                message: format!("failed to attach {key}: {error}"),
                            }).await;
                        }
                    }
                }
            }
        }
    }
}

fn spawn_connection(
    key: String,
    discovered: DiscoveredInstance,
    generation: u64,
    event_tx: &mpsc::Sender<UiEvent>,
    connection_tx: &mpsc::UnboundedSender<ConnectionEvent>,
) -> Connection {
    let (subscription_tx, subscription_rx) = mpsc::unbounded_channel();
    let task_event_tx = event_tx.clone();
    let task_connection_tx = connection_tx.clone();
    let task_key = key.clone();
    let task = tokio::spawn(async move {
        connection_task(
            task_key,
            discovered,
            generation,
            task_event_tx,
            task_connection_tx,
            subscription_rx,
        )
        .await;
    });
    Connection {
        id: None,
        subscription: Subscription::Summary,
        subscription_tx,
        task,
        generation,
    }
}

async fn connection_task(
    key: String,
    discovered: DiscoveredInstance,
    generation: u64,
    event_tx: mpsc::Sender<UiEvent>,
    connection_tx: mpsc::UnboundedSender<ConnectionEvent>,
    mut subscriptions: mpsc::UnboundedReceiver<Subscription>,
) {
    let connected = tokio::time::timeout(
        CONNECT_TIMEOUT,
        TelemetryClient::connect(&discovered, Subscription::Summary),
    )
    .await;
    let client = match connected {
        Ok(Ok(client)) => client,
        Ok(Err(error)) => {
            let _ = connection_tx.send(ConnectionEvent::Ended {
                key,
                generation,
                id: None,
                error: Some(error.to_string()),
            });
            return;
        }
        Err(_) => {
            let _ = connection_tx.send(ConnectionEvent::Ended {
                key,
                generation,
                id: None,
                error: Some("telemetry connection timed out".to_owned()),
            });
            return;
        }
    };

    let hello = client.hello().clone();
    let id = hello.instance.id.clone();
    let _ = event_tx.send(hello_ui_event(&hello)).await;
    let _ = connection_tx.send(ConnectionEvent::Identified {
        key: key.clone(),
        generation,
        id: id.clone(),
    });
    let (_, mut reader, mut writer) = client.into_parts();
    let mut starts = HashMap::new();
    let error = loop {
        tokio::select! {
            subscription = subscriptions.recv() => {
                let Some(subscription) = subscription else {
                    break None;
                };
                if subscription == Subscription::Summary {
                    starts.clear();
                }
                if let Err(error) = writer.subscribe(subscription).await {
                    break Some(error.to_string());
                }
            }
            message = reader.next_message() => {
                match message {
                    Ok(message) => {
                        for event in server_ui_events(message, &id, &mut starts) {
                            if event_tx.send(event).await.is_err() {
                                return;
                            }
                        }
                    }
                    Err(error) => break Some(error.to_string()),
                }
            }
        }
    };
    let _ = connection_tx.send(ConnectionEvent::Ended {
        key,
        generation,
        id: Some(id),
        error,
    });
}

fn update_subscriptions(connections: &mut HashMap<String, Connection>, selected: Option<&str>) {
    for connection in connections.values_mut() {
        update_subscription(connection, selected);
    }
}

fn update_subscription(connection: &mut Connection, selected: Option<&str>) {
    let desired = if connection
        .id
        .as_deref()
        .is_some_and(|id| Some(id) == selected)
    {
        Subscription::Detail
    } else {
        Subscription::Summary
    };
    if desired != connection.subscription {
        connection.subscription = desired;
        let _ = connection.subscription_tx.send(desired);
    }
}

fn hello_ui_event(hello: &Hello) -> UiEvent {
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
            endpoint: descriptor.endpoint.clone(),
            config_summary: descriptor.config_summary.clone(),
            telemetry_interval_ms: descriptor.telemetry_interval_ms,
        },
        lifecycle: Lifecycle::from_label(&hello.lifecycle),
        snapshot: None,
    }
}

fn server_ui_events(
    message: ServerMessage,
    id: &str,
    starts: &mut HashMap<u64, AccessRecord>,
) -> Vec<UiEvent> {
    match message {
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
            let mut events = Vec::with_capacity(2);
            if event.kind == RuntimeKind::Lifecycle
                && let Some(state) = event.message.split(':').next()
            {
                events.push(UiEvent::Lifecycle {
                    id: id.to_owned(),
                    lifecycle: Lifecycle::from_label(state),
                });
            }
            events.push(UiEvent::Runtime {
                id: id.to_owned(),
                record: runtime_ui_value(event),
            });
            events
        }
        ServerMessage::AccessStart(start) => {
            let record = access_start_ui_value(start);
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
        tcp_rx: value.tcp_rx,
        tcp_tx: value.tcp_tx,
        udp_rx: value.udp_rx,
        udp_tx: value.udp_tx,
        tcp_active: value.tcp_active,
        udp_active: value.udp_active,
        link_tcp: value.link_tcp,
        link_udp: value.link_udp,
        link_pairs: value.link_pairs,
        up_tcp: value.up_tcp,
        up_udp: value.up_udp,
        down_tcp: value.down_tcp,
        down_udp: value.down_udp,
        pool_active: value.pool_active,
        ping_ms: value.ping_ms,
        cpu_percent: value.cpu_percent,
        rss_bytes: value.rss_bytes,
        open_fds: value.open_fds,
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

fn access_start_ui_value(value: AccessStarted) -> AccessRecord {
    let route = value.path.unwrap_or_else(|| {
        let up = value.uplink.as_deref().unwrap_or("?");
        let down = value.downlink.as_deref().unwrap_or("?");
        format!("up:{up} down:{down}")
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
        client: value.client,
        path_peers: value.path_peers,
        route,
        target: Some(value.target),
        ..AccessRecord::default()
    }
}

fn access_finish_ui_value(
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
        uplink,
        downlink,
        path,
        upload_bytes,
        download_bytes,
        outcome,
        error,
    } = value;
    let mut record = starts.remove(&id).unwrap_or_else(|| {
        access_start_ui_value(AccessStarted {
            id,
            timestamp_ms,
            protocol,
            flow_id,
            client,
            path_peers,
            target,
            uplink,
            downlink,
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

fn is_benign_access_end(message: &str) -> bool {
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

#[cfg(test)]
#[path = "../tests/tui/client.rs"]
mod tests;
