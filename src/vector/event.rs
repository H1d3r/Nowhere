// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

//! Portal-compatible Vector telemetry.

use std::sync::Arc;
use tokio_util::sync::CancellationToken;

use crate::common::report_interval;
use crate::telemetry::Checkpoint;

use super::VectorInner;

pub(super) async fn event_loop(vector: Arc<VectorInner>, shutdown: CancellationToken) {
    loop {
        vector.client.refresh_latency().await;
        let checkpoint = Checkpoint::capture(
            vector.config.checkpoint_mode(),
            vector.client.ping_ms(),
            &vector.stats,
        );
        vector.logger.event(format_args!("{checkpoint}"));
        tokio::select! {
            _ = shutdown.cancelled() => return,
            _ = tokio::time::sleep(report_interval()) => {}
        }
    }
}

pub(super) async fn telemetry_loop(vector: Arc<VectorInner>, shutdown: CancellationToken) {
    let mut interval = tokio::time::interval(vector.telemetry_interval);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = shutdown.cancelled() => return,
            _ = interval.tick() => {
                vector.client.refresh_latency().await;
                vector.telemetry.capture_and_publish(
                    &vector.stats,
                    vector.client.ping_ms(),
                );
            }
        }
    }
}
