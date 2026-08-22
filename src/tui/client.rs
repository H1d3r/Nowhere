// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

//! Local telemetry-to-view-model adapter.
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

mod adapter;

#[cfg(test)]
use self::adapter::{access_finish_ui_value, access_start_ui_value, is_benign_access_end};
use self::adapter::{hello_ui_event, server_ui_events};

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

/// Starts registry discovery and one read-only loopback connection per instance.
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
                            .map(|instance| instance.registry_name.clone())
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
                            let key = instance.registry_name.clone();
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

#[cfg(test)]
#[path = "../tests/tui/client.rs"]
mod tests;
