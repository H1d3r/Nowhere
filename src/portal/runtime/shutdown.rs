// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

//! Portal admission shutdown, relay draining, and bounded task cleanup.

use super::*;
use quinn::VarInt;
use tokio::time::{Instant, timeout_at};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ShutdownOutcome {
    Drained,
    Timeout,
    Forced,
}

impl ShutdownOutcome {
    fn life_reason(self) -> LifeReason {
        match self {
            Self::Drained => LifeReason::Drained,
            Self::Timeout => LifeReason::Timeout,
            Self::Forced => LifeReason::Forced,
        }
    }
}

impl RunningPortal {
    pub(super) async fn shutdown(self, trigger: ShutdownTrigger) -> Result<()> {
        let RunningPortal {
            portal,
            mut signals,
            endpoints,
            mut quic_listeners,
            mut tcp_listener_tasks,
            mut auxiliary_tasks,
            mut telemetry_tasks,
            telemetry_shutdown,
            stop_accepting,
            force_shutdown,
        } = self;
        let deadline = Instant::now() + portal.inner.runtime.shutdown_timeout;

        portal.inner.pairing.close_admission();
        portal.inner.ready_gate.close();
        portal.inner.drain.cancel();
        portal.inner.relay_tasks.close();
        for endpoint in &endpoints {
            endpoint.set_server_config(None);
        }
        stop_accepting.cancel();
        portal.inner.lifecycle.transition(
            &portal.inner.logger,
            LifeState::Draining,
            trigger.reason,
        );
        portal
            .inner
            .telemetry
            .set_lifecycle(LifeState::Draining.to_string(), trigger.reason.to_string());

        let drain = async {
            portal.inner.pairing.begin_drain().await;
            portal.inner.relay_tasks.wait().await;
        };
        let mut outcome = tokio::select! {
            biased;
            signal = signals.recv() => {
                match signal {
                    Ok(_) => ShutdownOutcome::Forced,
                    Err(error) => {
                        portal.inner.logger.error(format_args!(
                            "portal::run: shutdown signal stream failed during drain: {error}"
                        ));
                        ShutdownOutcome::Forced
                    }
                }
            }
            result = timeout_at(deadline, drain) => match result {
                Ok(()) => ShutdownOutcome::Drained,
                Err(_) => ShutdownOutcome::Timeout,
            }
        };

        force_shutdown.cancel();
        portal.inner.outbound.close(deadline).await;
        for endpoint in &endpoints {
            endpoint.close(VarInt::from_u32(0), b"");
        }
        portal.inner.connection_tasks.close();
        if outcome != ShutdownOutcome::Drained {
            portal.inner.relay_tasks.abort_all();
            portal.inner.connection_tasks.abort_all();
            quic_listeners.abort_all();
            tcp_listener_tasks.abort_all();
            auxiliary_tasks.abort_all();
        }

        let mut endpoint_tasks = JoinSet::new();
        for endpoint in &endpoints {
            let endpoint = endpoint.clone();
            endpoint_tasks.spawn(async move {
                endpoint.wait_idle().await;
            });
        }

        let cleanup = async {
            portal.inner.pairing.cancel_all().await;
            while endpoint_tasks.join_next().await.is_some() {}
            while quic_listeners.join_next().await.is_some() {}
            while tcp_listener_tasks.join_next().await.is_some() {}
            while auxiliary_tasks.join_next().await.is_some() {}
            portal.inner.connection_tasks.wait().await;
            portal.inner.relay_tasks.wait().await;
        };
        let cleanup_deadline = if outcome == ShutdownOutcome::Forced {
            Instant::now()
        } else {
            deadline
        };
        if timeout_at(cleanup_deadline, cleanup).await.is_err() {
            if outcome == ShutdownOutcome::Drained {
                outcome = ShutdownOutcome::Timeout;
            }
            endpoint_tasks.abort_all();
            quic_listeners.abort_all();
            tcp_listener_tasks.abort_all();
            auxiliary_tasks.abort_all();
            portal.inner.connection_tasks.abort_all();
            portal.inner.relay_tasks.abort_all();
            while endpoint_tasks.join_next().await.is_some() {}
            while quic_listeners.join_next().await.is_some() {}
            while tcp_listener_tasks.join_next().await.is_some() {}
            while auxiliary_tasks.join_next().await.is_some() {}
            portal.inner.connection_tasks.wait().await;
            portal.inner.relay_tasks.wait().await;
            portal.inner.pairing.cancel_all().await;
        }

        if let Some(rate) = &portal.inner.rate_limiter {
            rate.reset();
        }
        portal.inner.lifecycle.transition(
            &portal.inner.logger,
            LifeState::Stopped,
            outcome.life_reason(),
        );
        portal.inner.telemetry.set_lifecycle(
            LifeState::Stopped.to_string(),
            outcome.life_reason().to_string(),
        );
        portal
            .inner
            .telemetry
            .capture_and_publish(&portal.inner.stats, portal.inner.outbound.ping_ms());
        tokio::task::yield_now().await;
        telemetry_shutdown.cancel();
        while telemetry_tasks.join_next().await.is_some() {}
        portal
            .inner
            .logger
            .info(format_args!("portal::run: portal shutdown complete"));
        portal.inner.logger.flush();

        match trigger.failure {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}
