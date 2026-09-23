// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

use super::*;

pub(super) const OFFLINE_RETENTION: Duration = Duration::from_secs(30);

/// One process and all state retained locally for it.
#[derive(Clone, Debug)]
pub struct InstanceView {
    pub meta: InstanceMeta,
    pub lifecycle: Lifecycle,
    pub online: bool,
    pub snapshot: Option<TelemetrySnapshot>,
    pub history: VecDeque<HistoryPoint>,
    pub access: VecDeque<AccessRecord>,
    pub runtime: VecDeque<RuntimeRecord>,
    pub dropped_events: u64,
    pub overwritten_events: u64,
    pub(super) offline_since: Option<Instant>,
}

impl InstanceView {
    pub(super) fn new(
        meta: InstanceMeta,
        lifecycle: Lifecycle,
        snapshot: Option<TelemetrySnapshot>,
    ) -> Self {
        Self {
            meta,
            lifecycle,
            online: true,
            snapshot,
            history: VecDeque::new(),
            access: VecDeque::with_capacity(FEED_CAPACITY),
            runtime: VecDeque::with_capacity(FEED_CAPACITY),
            dropped_events: 0,
            overwritten_events: 0,
            offline_since: None,
        }
    }

    pub(super) fn update_snapshot(&mut self, snapshot: TelemetrySnapshot) {
        if let Some(previous) = self.snapshot.as_ref() {
            let sample_clock = snapshot.sample_clock_ms();
            let previous_clock = previous.sample_clock_ms();
            let elapsed_ms = sample_clock.saturating_sub(previous_clock);
            if elapsed_ms == 0 || snapshot.counter_reset_from(previous) {
                if sample_clock < previous_clock || snapshot.counter_reset_from(previous) {
                    self.history.clear();
                }
            } else {
                let rate = |new: u64, old: u64| {
                    new.saturating_sub(old) as f64 * 8_000.0 / elapsed_ms as f64
                };
                self.history.push_back(HistoryPoint {
                    timestamp_ms: sample_clock,
                    upload_bps: rate(snapshot.upload_bytes(), previous.upload_bytes()),
                    download_bps: rate(snapshot.download_bytes(), previous.download_bytes()),
                    tcp_bps: rate(snapshot.tcp_bytes(), previous.tcp_bytes()),
                    udp_bps: rate(snapshot.udp_bytes(), previous.udp_bytes()),
                    tls_bps: rate(snapshot.tls_bytes(), previous.tls_bytes()),
                    quic_bps: rate(snapshot.quic_bytes(), previous.quic_bytes()),
                    tcp_active: snapshot.tcp_active.max(0),
                    udp_active: snapshot.udp_active.max(0),
                    tls_links: snapshot.tls_carriers_active,
                    quic_links: snapshot.quic_carriers_active,
                    cpu_percent: snapshot.cpu_percent.unwrap_or_default().max(0.0),
                    rss_bytes: snapshot.rss_bytes.unwrap_or_default(),
                });
                while self.history.front().is_some_and(|point| {
                    sample_clock.saturating_sub(point.timestamp_ms) > HISTORY_WINDOW_MS
                }) {
                    self.history.pop_front();
                }
            }
        }
        self.online = true;
        self.offline_since = None;
        self.snapshot = Some(snapshot);
    }

    pub(super) fn push_access(&mut self, record: AccessRecord) -> bool {
        if record.phase == AccessPhase::Finish
            && let Some(existing) = self
                .access
                .iter_mut()
                .rev()
                .find(|existing| existing.event_id == record.event_id)
        {
            *existing = record;
            return false;
        }
        if push_bounded(&mut self.access, record) {
            self.overwritten_events = self.overwritten_events.saturating_add(1);
        }
        true
    }

    pub(super) fn push_runtime(&mut self, record: RuntimeRecord) {
        if push_bounded(&mut self.runtime, record) {
            self.overwritten_events = self.overwritten_events.saturating_add(1);
        }
    }

    pub(super) fn mark_offline(&mut self, now: Instant) {
        self.online = false;
        self.offline_since.get_or_insert(now);
    }

    pub(super) fn expired(&self, now: Instant) -> bool {
        self.offline_since
            .is_some_and(|since| now.saturating_duration_since(since) >= OFFLINE_RETENTION)
    }

    pub fn latest_history(&self) -> HistoryPoint {
        self.history.back().copied().unwrap_or_default()
    }
}

fn push_bounded<T>(queue: &mut VecDeque<T>, value: T) -> bool {
    let overwritten = queue.len() == FEED_CAPACITY;
    if queue.len() == FEED_CAPACITY {
        queue.pop_front();
    }
    queue.push_back(value);
    overwritten
}
