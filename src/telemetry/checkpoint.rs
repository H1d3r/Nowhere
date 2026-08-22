// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

//! Stable EVENT checkpoint formatting for external consumers.

use std::fmt;
use std::sync::atomic::Ordering;

use crate::transport::Stats;

pub(crate) struct Checkpoint {
    mode: u8,
    ping_ms: u64,
    tcp_active: i32,
    udp_active: i32,
    tcp_rx: u64,
    tcp_tx: u64,
    udp_rx: u64,
    udp_tx: u64,
}

impl Checkpoint {
    pub(crate) fn capture(mode: u8, ping_ms: u64, stats: &Stats) -> Self {
        Self {
            mode,
            ping_ms,
            tcp_active: stats.tcp_active.load(Ordering::Relaxed),
            udp_active: stats.udp_active.load(Ordering::Relaxed),
            tcp_rx: stats.tcp_rx.load(Ordering::Relaxed),
            tcp_tx: stats.tcp_tx.load(Ordering::Relaxed),
            udp_rx: stats.udp_rx.load(Ordering::Relaxed),
            udp_tx: stats.udp_tx.load(Ordering::Relaxed),
        }
    }
}

impl fmt::Display for Checkpoint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "CHECK_POINT|MODE={}|PING={}ms|POOL=0|TCPS={}|UDPS={}|TCPRX={}|TCPTX={}|UDPRX={}|UDPTX={}",
            self.mode,
            self.ping_ms,
            self.tcp_active,
            self.udp_active,
            self.tcp_rx,
            self.tcp_tx,
            self.udp_rx,
            self.udp_tx,
        )
    }
}

#[cfg(test)]
#[path = "../tests/telemetry/checkpoint.rs"]
mod tests;
