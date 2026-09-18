// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

//! In-process structured telemetry publisher.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::sync::{broadcast, watch};

use crate::protocol::Carrier;
use crate::transport::Stats;

use super::process::{ProcessSampler, now_unix_ms, process_uptime_ms};
use super::wire::{
    AccessFinished, AccessOutcome, AccessStart, AccessStarted, InstanceDescriptor, InstanceRole,
    LifecycleSnapshot, RuntimeEvent, RuntimeKind, RuntimeLevel, ServerMessage, TelemetrySnapshot,
};

const EVENT_CAPACITY: usize = 1_024;

/// The in-process publisher shared by runtime orchestration and every flow.
pub(crate) struct TelemetryHub {
    descriptor: InstanceDescriptor,
    privacy: Option<super::privacy::Privacy>,
    detail_clients: AtomicU64,
    event_sequence: Mutex<u64>,
    lifecycle: watch::Sender<LifecycleSnapshot>,
    snapshots: watch::Sender<TelemetrySnapshot>,
    events: broadcast::Sender<ServerMessage>,
    next_sequence: AtomicU64,
    next_access_id: AtomicU64,
    started: Instant,
    process_sampler: Mutex<ProcessSampler>,
    unavailable_reason: Option<String>,
}

impl TelemetryHub {
    pub(crate) fn new(descriptor: InstanceDescriptor) -> Arc<Self> {
        Self::with_availability(descriptor, None)
    }

    pub(crate) fn for_current_process(
        role: InstanceRole,
        endpoint: impl Into<String>,
        config_summary: impl Into<String>,
        telemetry_interval: Duration,
    ) -> Arc<Self> {
        let endpoint = endpoint.into();
        let config_summary = config_summary.into();
        match InstanceDescriptor::current(
            role,
            endpoint.clone(),
            config_summary.clone(),
            telemetry_interval,
        ) {
            Ok(descriptor) => Self::new(descriptor),
            Err(error) => Self::with_availability(
                InstanceDescriptor::unavailable(role, endpoint, config_summary, telemetry_interval),
                Some(error.to_string()),
            ),
        }
    }

    fn with_availability(
        descriptor: InstanceDescriptor,
        unavailable_reason: Option<String>,
    ) -> Arc<Self> {
        let privacy = super::privacy::Privacy::new().ok();
        let unavailable_reason = unavailable_reason.or_else(|| {
            privacy
                .is_none()
                .then(|| "telemetry entropy unavailable".to_owned())
        });
        let (snapshots, _) = watch::channel(TelemetrySnapshot::default());
        let (lifecycle, _) = watch::channel(LifecycleSnapshot::default());
        let (events, _) = broadcast::channel(EVENT_CAPACITY);
        Arc::new(Self {
            descriptor,
            privacy,
            detail_clients: AtomicU64::new(0),
            event_sequence: Mutex::new(0),
            lifecycle,
            snapshots,
            events,
            next_sequence: AtomicU64::new(1),
            next_access_id: AtomicU64::new(1),
            started: Instant::now(),
            process_sampler: Mutex::new(ProcessSampler::default()),
            unavailable_reason,
        })
    }

    pub(crate) fn descriptor(&self) -> &InstanceDescriptor {
        &self.descriptor
    }

    pub(crate) fn detail_guard(self: &Arc<Self>) -> DetailGuard {
        self.detail_clients.fetch_add(1, Ordering::Relaxed);
        DetailGuard(self.clone())
    }

    fn publish_event(&self, mut event: ServerMessage) {
        let mut sequence = self
            .event_sequence
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        *sequence += 1;
        match &mut event {
            ServerMessage::RuntimeEvent(e) => e.sequence = *sequence,
            ServerMessage::AccessStart(e) => e.sequence = *sequence,
            ServerMessage::AccessFinish(e) => e.sequence = *sequence,
            _ => return,
        }
        let _ = self.events.send(event);
    }

    pub(crate) fn unavailable_reason(&self) -> Option<&str> {
        self.unavailable_reason.as_deref()
    }

    pub(crate) fn set_lifecycle(&self, state: impl Into<String>, reason: impl Into<String>) {
        let state = state.into();
        let state = if matches!(
            state.as_str(),
            "STARTING" | "READY" | "DRAINING" | "STOPPED"
        ) {
            state
        } else {
            "STOPPED".to_owned()
        };
        let reason = reason.into();
        let reason = super::privacy::lifecycle_reason(&reason).to_owned();
        self.lifecycle.send_replace(LifecycleSnapshot {
            state: state.clone(),
            reason: reason.clone(),
            timestamp_ms: now_unix_ms(),
        });
        self.emit_runtime(RuntimeEvent::new(
            RuntimeLevel::Info,
            RuntimeKind::Lifecycle,
            format!("{state}: {reason}"),
        ));
    }

    pub(crate) fn lifecycle_receiver(&self) -> watch::Receiver<LifecycleSnapshot> {
        self.lifecycle.subscribe()
    }

    /// Atomically captures the existing transport counters plus local process
    /// resources, then wakes every connected summary/detail subscriber.
    pub(crate) fn capture_and_publish(&self, stats: &Stats, ping_ms: u64) {
        let process = self
            .process_sampler
            .lock()
            .unwrap_or_else(|lock| lock.into_inner())
            .sample();
        let tls_carriers_active = stats.link_tcp.load(Ordering::Relaxed);
        let quic_carriers_active = stats.link_udp.load(Ordering::Relaxed);
        let snapshot = TelemetrySnapshot {
            sequence: self.next_sequence.fetch_add(1, Ordering::Relaxed),
            timestamp_ms: now_unix_ms(),
            uptime_ms: process_uptime_ms(self.descriptor.incarnation)
                .unwrap_or_else(|| self.started.elapsed().as_millis().min(u64::MAX as u128) as u64),
            tcp_logical_up: stats.tcp_rx.load(Ordering::Relaxed),
            tcp_logical_down: stats.tcp_tx.load(Ordering::Relaxed),
            udp_logical_up: stats.udp_rx.load(Ordering::Relaxed),
            udp_logical_down: stats.udp_tx.load(Ordering::Relaxed),
            tls_payload_up: stats.up_tcp.load(Ordering::Relaxed),
            tls_payload_down: stats.down_tcp.load(Ordering::Relaxed),
            quic_payload_up: stats.up_udp.load(Ordering::Relaxed),
            quic_payload_down: stats.down_udp.load(Ordering::Relaxed),
            tcp_active: i64::from(stats.tcp_active.load(Ordering::Relaxed)),
            udp_active: i64::from(stats.udp_active.load(Ordering::Relaxed)),
            tls_carriers_active,
            quic_carriers_active,
            ..TelemetrySnapshot::default()
        };
        let snapshot = TelemetrySnapshot {
            ping_ms,
            cpu_percent: process.cpu_percent,
            rss_bytes: process.rss_bytes,
            open_fds: process.open_fds,
            ..snapshot
        };
        self.snapshots.send_replace(snapshot);
    }

    pub(crate) fn emit_runtime(&self, mut event: RuntimeEvent) {
        if self.detail_clients.load(Ordering::Relaxed) == 0 {
            return;
        }
        event.message = super::privacy::runtime_message(event.kind, event.level, &event.message);
        event.client = event.client.map(|v| {
            self.privacy
                .as_ref()
                .expect("available privacy")
                .alias("client", &v)
        });
        self.publish_event(ServerMessage::RuntimeEvent(event));
    }

    pub(crate) fn start_access(
        self: &Arc<Self>,
        build: impl FnOnce() -> AccessStart,
    ) -> AccessSpan {
        // A broadcast receiver cannot recover an AccessStart emitted before it
        // subscribed. Avoid building and cloning path strings when no detail
        // client can observe this flow; this is especially important for
        // high-rate short connections.
        if self.detail_clients.load(Ordering::Relaxed) == 0 || self.privacy.is_none() {
            return AccessSpan::disabled(Arc::clone(self));
        }
        let mut start = build();
        start.id = self.next_access_id.fetch_add(1, Ordering::Relaxed);
        if start.timestamp_ms == 0 {
            start.timestamp_ms = now_unix_ms();
        }
        let _ = (&start.flow_id, &start.session_tag, &start.path);
        let started_at = Instant::now();
        let privacy = self.privacy.as_ref().expect("available privacy");
        let started = AccessStarted {
            sequence: 0,
            truncated: start.path_peers.len() > 16,
            id: start.id,
            timestamp_ms: start.timestamp_ms,
            protocol: start.protocol,
            flow_id: None,
            session_tag: None,
            client: start.client.map(|v| privacy.alias("client", &v)),
            path_peers: start
                .path_peers
                .iter()
                .take(16)
                .map(|v| privacy.alias("peer", v))
                .collect(),
            target: super::privacy::target(&start.target),
            initial_uplink: start.initial_uplink.map(carrier_name).map(str::to_owned),
            initial_downlink: start.initial_downlink.map(carrier_name).map(str::to_owned),
            path: None,
        };
        self.publish_event(ServerMessage::AccessStart(started.clone()));
        AccessSpan::new(Arc::clone(self), started, started_at)
    }

    pub(crate) fn snapshot_receiver(&self) -> watch::Receiver<TelemetrySnapshot> {
        self.snapshots.subscribe()
    }

    pub(crate) fn event_receiver(&self) -> broadcast::Receiver<ServerMessage> {
        self.events.subscribe()
    }

    fn finish_access(
        &self,
        started: &AccessStarted,
        started_at: Instant,
        bytes: (u64, u64),
        completion: (AccessOutcome, Option<String>),
    ) {
        let (upload_bytes, download_bytes) = bytes;
        let (outcome, error) = completion;
        self.publish_event(ServerMessage::AccessFinish(AccessFinished {
            sequence: 0,
            truncated: started.truncated,
            id: started.id,
            timestamp_ms: now_unix_ms(),
            duration_ms: started_at.elapsed().as_millis().min(u64::MAX as u128) as u64,
            protocol: started.protocol,
            flow_id: started.flow_id,
            session_tag: started.session_tag.clone(),
            client: started.client.clone(),
            path_peers: started.path_peers.clone(),
            target: started.target.clone(),
            initial_uplink: started.initial_uplink.clone(),
            initial_downlink: started.initial_downlink.clone(),
            path: started.path.clone(),
            upload_bytes,
            download_bytes,
            outcome,
            error: error.map(|value| super::privacy::error_reason(&value).to_owned()),
        }));
    }
}

/// Cancellation-safe per-flow accounting. An unfinished span emits one
/// `cancelled` completion when dropped.
pub(crate) struct AccessSpan {
    hub: Arc<TelemetryHub>,
    started: Option<AccessStarted>,
    started_at: Instant,
    upload_bytes: AtomicU64,
    download_bytes: AtomicU64,
    finished: AtomicBool,
}

impl AccessSpan {
    fn new(hub: Arc<TelemetryHub>, started: AccessStarted, started_at: Instant) -> Self {
        Self {
            hub,
            started: Some(started),
            started_at,
            upload_bytes: AtomicU64::new(0),
            download_bytes: AtomicU64::new(0),
            finished: AtomicBool::new(false),
        }
    }

    fn disabled(hub: Arc<TelemetryHub>) -> Self {
        Self {
            hub,
            started: None,
            started_at: Instant::now(),
            upload_bytes: AtomicU64::new(0),
            download_bytes: AtomicU64::new(0),
            finished: AtomicBool::new(false),
        }
    }

    pub(crate) fn add_upload(&self, bytes: u64) {
        if self.started.is_some() {
            self.upload_bytes.fetch_add(bytes, Ordering::Relaxed);
        }
    }

    pub(crate) fn add_download(&self, bytes: u64) {
        if self.started.is_some() {
            self.download_bytes.fetch_add(bytes, Ordering::Relaxed);
        }
    }

    pub(crate) fn finish(self, outcome: AccessOutcome, error: Option<String>) {
        self.finish_once(outcome, error);
    }

    fn finish_once(&self, outcome: AccessOutcome, error: Option<String>) {
        if self.finished.swap(true, Ordering::AcqRel) {
            return;
        }
        let Some(started) = &self.started else {
            return;
        };
        self.hub.finish_access(
            started,
            self.started_at,
            (
                self.upload_bytes.load(Ordering::Relaxed),
                self.download_bytes.load(Ordering::Relaxed),
            ),
            (outcome, error),
        );
    }
}

impl Drop for AccessSpan {
    fn drop(&mut self) {
        self.finish_once(AccessOutcome::Cancelled, None);
    }
}

fn carrier_name(carrier: Carrier) -> &'static str {
    match carrier {
        Carrier::TlsTcp => "tcp",
        Carrier::Quic => "udp",
    }
}

#[cfg(test)]
#[path = "../tests/telemetry/hub.rs"]
mod tests;

pub(crate) struct DetailGuard(Arc<TelemetryHub>);
impl Drop for DetailGuard {
    fn drop(&mut self) {
        self.0.detail_clients.fetch_sub(1, Ordering::Relaxed);
    }
}
