// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

//! Periodic event telemetry emitted by a running portal.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use tokio_util::sync::CancellationToken;

use super::PortalInner;

pub(super) async fn event_loop(portal: Arc<PortalInner>, shutdown: CancellationToken) {
    loop {
        portal.outbound.refresh_latency().await;
        portal.logger.event(format_args!(
            "CHECK_POINT|MODE={}|PING={}ms|POOL={}|TCPS={}|UDPS={}|TCPRX={}|TCPTX={}|UDPRX={}|UDPTX={}",
            portal.network_mode.checkpoint_value(),
            portal.outbound.ping_ms(),
            portal.pool_active.load(Ordering::Relaxed),
            portal.stats.tcp_active.load(Ordering::Relaxed),
            portal.stats.udp_active.load(Ordering::Relaxed),
            portal.stats.tcp_rx.load(Ordering::Relaxed),
            portal.stats.tcp_tx.load(Ordering::Relaxed),
            portal.stats.udp_rx.load(Ordering::Relaxed),
            portal.stats.udp_tx.load(Ordering::Relaxed),
        ));
        portal.logger.debug(format_args!(
            "LINK_STATUS|TCP={}|UDP={}|PAIRS={}|UPTCP={}|UPUDP={}|DOWNTCP={}|DOWNUDP={}",
            portal.stats.link_tcp.load(Ordering::Relaxed),
            portal.stats.link_udp.load(Ordering::Relaxed),
            portal.stats.link_pairs.load(Ordering::Relaxed),
            portal.stats.up_tcp.load(Ordering::Relaxed),
            portal.stats.up_udp.load(Ordering::Relaxed),
            portal.stats.down_tcp.load(Ordering::Relaxed),
            portal.stats.down_udp.load(Ordering::Relaxed),
        ));

        tokio::select! {
            _ = shutdown.cancelled() => return,
            _ = tokio::time::sleep(portal.runtime.report_interval) => {}
        }
    }
}

pub(super) async fn telemetry_loop(portal: Arc<PortalInner>, shutdown: CancellationToken) {
    let mut interval = tokio::time::interval(portal.runtime.telemetry_interval);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = shutdown.cancelled() => return,
            _ = interval.tick() => {
                portal.outbound.refresh_latency().await;
                portal.telemetry.capture_and_publish(
                    &portal.stats,
                    portal.pool_active.load(Ordering::Relaxed),
                    portal.outbound.ping_ms(),
                );
            }
        }
    }
}
