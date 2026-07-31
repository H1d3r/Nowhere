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
        let (snapshots, _) = watch::channel(TelemetrySnapshot::default());
        let (lifecycle, _) = watch::channel(LifecycleSnapshot::default());
        let (events, _) = broadcast::channel(EVENT_CAPACITY);
        Arc::new(Self {
            descriptor,
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

    pub(crate) fn unavailable_reason(&self) -> Option<&str> {
        self.unavailable_reason.as_deref()
    }

    pub(crate) fn set_lifecycle(&self, state: impl Into<String>, reason: impl Into<String>) {
        let state = state.into();
        let reason = reason.into();
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
    pub(crate) fn capture_and_publish(&self, stats: &Stats, pool_active: u64) {
        let process = self
            .process_sampler
            .lock()
            .unwrap_or_else(|lock| lock.into_inner())
            .sample();
        let link_tcp = stats.link_tcp.load(Ordering::Relaxed);
        let link_udp = stats.link_udp.load(Ordering::Relaxed);
        let stored_pairs = stats.link_pairs.load(Ordering::Relaxed);
        let link_pairs = match self.descriptor.role {
            InstanceRole::Vector => u64::from(link_tcp != 0 && link_udp != 0),
            InstanceRole::Portal => stored_pairs,
        };
        let snapshot = TelemetrySnapshot {
            sequence: self.next_sequence.fetch_add(1, Ordering::Relaxed),
            timestamp_ms: now_unix_ms(),
            uptime_ms: process_uptime_ms(self.descriptor.start_ticks)
                .unwrap_or_else(|| self.started.elapsed().as_millis().min(u64::MAX as u128) as u64),
            tcp_rx: stats.tcp_rx.load(Ordering::Relaxed),
            tcp_tx: stats.tcp_tx.load(Ordering::Relaxed),
            udp_rx: stats.udp_rx.load(Ordering::Relaxed),
            udp_tx: stats.udp_tx.load(Ordering::Relaxed),
            tcp_active: i64::from(stats.tcp_active.load(Ordering::Relaxed)),
            udp_active: i64::from(stats.udp_active.load(Ordering::Relaxed)),
            link_tcp,
            link_udp,
            link_pairs,
            up_tcp: stats.up_tcp.load(Ordering::Relaxed),
            up_udp: stats.up_udp.load(Ordering::Relaxed),
            down_tcp: stats.down_tcp.load(Ordering::Relaxed),
            down_udp: stats.down_udp.load(Ordering::Relaxed),
            pool_active,
            cpu_percent: process.cpu_percent,
            rss_bytes: process.rss_bytes,
            open_fds: process.open_fds,
        };
        self.snapshots.send_replace(snapshot);
    }

    pub(crate) fn emit_runtime(&self, event: RuntimeEvent) {
        let _ = self.events.send(ServerMessage::RuntimeEvent(event));
    }

    pub(crate) fn start_access(self: &Arc<Self>, mut start: AccessStart) -> AccessSpan {
        if start.id == 0 {
            start.id = self.next_access_id.fetch_add(1, Ordering::Relaxed);
        }
        if start.timestamp_ms == 0 {
            start.timestamp_ms = now_unix_ms();
        }
        let started_at = Instant::now();
        let started = AccessStarted {
            id: start.id,
            timestamp_ms: start.timestamp_ms,
            protocol: start.protocol,
            flow_id: start.flow_id,
            client: start.client,
            path_peers: start.path_peers,
            target: start.target,
            uplink: start.uplink.map(carrier_name).map(str::to_owned),
            downlink: start.downlink.map(carrier_name).map(str::to_owned),
            path: start.path,
        };
        let _ = self
            .events
            .send(ServerMessage::AccessStart(started.clone()));
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
        upload_bytes: u64,
        download_bytes: u64,
        outcome: AccessOutcome,
        error: Option<String>,
    ) {
        let _ = self
            .events
            .send(ServerMessage::AccessFinish(AccessFinished {
                id: started.id,
                timestamp_ms: now_unix_ms(),
                duration_ms: started_at.elapsed().as_millis().min(u64::MAX as u128) as u64,
                protocol: started.protocol,
                flow_id: started.flow_id,
                client: started.client.clone(),
                path_peers: started.path_peers.clone(),
                target: started.target.clone(),
                uplink: started.uplink.clone(),
                downlink: started.downlink.clone(),
                path: started.path.clone(),
                upload_bytes,
                download_bytes,
                outcome,
                error,
            }));
    }
}

/// Cancellation-safe per-flow accounting. An unfinished span emits one
/// `cancelled` completion when dropped.
pub(crate) struct AccessSpan {
    hub: Arc<TelemetryHub>,
    started: AccessStarted,
    started_at: Instant,
    upload_bytes: AtomicU64,
    download_bytes: AtomicU64,
    finished: AtomicBool,
}

impl AccessSpan {
    fn new(hub: Arc<TelemetryHub>, started: AccessStarted, started_at: Instant) -> Self {
        Self {
            hub,
            started,
            started_at,
            upload_bytes: AtomicU64::new(0),
            download_bytes: AtomicU64::new(0),
            finished: AtomicBool::new(false),
        }
    }

    pub(crate) fn add_upload(&self, bytes: u64) {
        self.upload_bytes.fetch_add(bytes, Ordering::Relaxed);
    }

    pub(crate) fn add_download(&self, bytes: u64) {
        self.download_bytes.fetch_add(bytes, Ordering::Relaxed);
    }

    pub(crate) fn finish(self, outcome: AccessOutcome, error: Option<String>) {
        self.finish_once(outcome, error);
    }

    fn finish_once(&self, outcome: AccessOutcome, error: Option<String>) {
        if self.finished.swap(true, Ordering::AcqRel) {
            return;
        }
        self.hub.finish_access(
            &self.started,
            self.started_at,
            self.upload_bytes.load(Ordering::Relaxed),
            self.download_bytes.load(Ordering::Relaxed),
            outcome,
            error,
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
