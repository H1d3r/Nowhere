// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

//! Atomic portal traffic and session counters.

use std::sync::atomic::{AtomicI32, AtomicU64, Ordering};

#[derive(Debug, Default)]
pub struct Stats {
    pub tcp_rx: AtomicU64,
    pub tcp_tx: AtomicU64,
    pub udp_rx: AtomicU64,
    pub udp_tx: AtomicU64,
    pub tcp_active: AtomicI32,
    pub udp_active: AtomicI32,
    pub link_tcp: AtomicU64,
    pub link_udp: AtomicU64,
    pub up_tcp: std::sync::Arc<AtomicU64>,
    pub up_udp: std::sync::Arc<AtomicU64>,
    pub down_tcp: std::sync::Arc<AtomicU64>,
    pub down_udp: std::sync::Arc<AtomicU64>,
}

impl Stats {
    pub fn add_session(&self, is_udp: bool) {
        if is_udp {
            self.udp_active.fetch_add(1, Ordering::Relaxed);
        } else {
            self.tcp_active.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn done_session(&self, is_udp: bool) {
        if is_udp {
            self.udp_active.fetch_sub(1, Ordering::Relaxed);
        } else {
            self.tcp_active.fetch_sub(1, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
#[path = "../tests/transport/stats.rs"]
mod tests;
