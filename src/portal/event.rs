// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

//! Periodic event telemetry emitted by a running portal.

use std::sync::Arc;
use tokio_util::sync::CancellationToken;

use super::PortalInner;
use crate::telemetry::Checkpoint;

pub(super) async fn event_loop(portal: Arc<PortalInner>, shutdown: CancellationToken) {
    loop {
        portal.outbound.refresh_latency().await;
        let checkpoint = Checkpoint::capture(
            portal.network_mode.checkpoint_value(),
            portal.outbound.ping_ms(),
            &portal.stats,
        );
        portal.logger.event(format_args!("{checkpoint}"));
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
                    portal.outbound.ping_ms(),
                );
            }
        }
    }
}
