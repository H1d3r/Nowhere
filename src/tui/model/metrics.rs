// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

//! Snapshot counters and chart-ready rate samples.

/// The visible chart window. Services do not retain history themselves.
pub const HISTORY_WINDOW_MS: u64 = 10 * 60 * 1_000;

/// One cumulative telemetry sample.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TelemetrySnapshot {
    pub sequence: u64,
    /// Wall-clock milliseconds reported by the service.
    pub timestamp_ms: u64,
    /// Monotonic milliseconds since this service process started.
    pub uptime_ms: u64,
    pub tcp_rx: u64,
    pub tcp_tx: u64,
    pub udp_rx: u64,
    pub udp_tx: u64,
    pub tcp_active: i64,
    pub udp_active: i64,
    pub link_tcp: u64,
    pub link_udp: u64,
    pub link_pairs: u64,
    pub up_tcp: u64,
    pub up_udp: u64,
    pub down_tcp: u64,
    pub down_udp: u64,
    pub pool_active: u64,
    pub ping_ms: u64,
    pub cpu_percent: Option<f64>,
    pub rss_bytes: Option<u64>,
    pub open_fds: Option<u64>,
}

impl TelemetrySnapshot {
    pub fn upload_bytes(&self) -> u64 {
        self.tcp_rx.saturating_add(self.udp_rx)
    }

    pub fn download_bytes(&self) -> u64 {
        self.tcp_tx.saturating_add(self.udp_tx)
    }

    pub fn tcp_bytes(&self) -> u64 {
        self.tcp_rx.saturating_add(self.tcp_tx)
    }

    pub fn udp_bytes(&self) -> u64 {
        self.udp_rx.saturating_add(self.udp_tx)
    }

    pub fn tls_bytes(&self) -> u64 {
        self.up_tcp.saturating_add(self.down_tcp)
    }

    pub fn quic_bytes(&self) -> u64 {
        self.up_udp.saturating_add(self.down_udp)
    }

    pub(super) fn counter_reset_from(&self, old: &Self) -> bool {
        [
            (self.upload_bytes(), old.upload_bytes()),
            (self.download_bytes(), old.download_bytes()),
            (self.tcp_bytes(), old.tcp_bytes()),
            (self.udp_bytes(), old.udp_bytes()),
            (self.tls_bytes(), old.tls_bytes()),
            (self.quic_bytes(), old.quic_bytes()),
        ]
        .into_iter()
        .any(|(new, old)| new < old)
    }

    pub(super) fn sample_clock_ms(&self) -> u64 {
        if self.uptime_ms == 0 {
            self.timestamp_ms
        } else {
            self.uptime_ms
        }
    }
}

/// Derived per-second data retained for charts.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HistoryPoint {
    pub timestamp_ms: u64,
    pub upload_bps: f64,
    pub download_bps: f64,
    pub tcp_bps: f64,
    pub udp_bps: f64,
    pub tls_bps: f64,
    pub quic_bps: f64,
    pub tcp_active: i64,
    pub udp_active: i64,
    pub tls_links: u64,
    pub quic_links: u64,
    pub pairs: u64,
    pub pool_active: u64,
    pub ping_ms: u64,
    pub cpu_percent: f64,
    pub rss_bytes: u64,
}
