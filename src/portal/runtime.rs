// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

//! Portal runtime orchestration, listener supervision, and bounded flow drain.

use anyhow::{Context, Result};
use quinn::Endpoint;
use tokio::net::TcpListener;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use crate::common::{LifeReason, LifeState, ShutdownSignals};
use crate::telemetry::TelemetryServer;

use super::listener::{accept_endpoint_loop, accept_tcp_loop, listen_endpoint, listen_tcp};
use super::{Portal, event};

mod binding;
mod shutdown;
#[cfg(test)]
use binding::{bind_carrier, io_error_is_family_unavailable};

struct ShutdownTrigger {
    reason: LifeReason,
    failure: Option<anyhow::Error>,
}

struct RunningPortal {
    portal: Portal,
    signals: ShutdownSignals,
    endpoints: Vec<Endpoint>,
    quic_listeners: JoinSet<()>,
    tcp_listener_tasks: JoinSet<()>,
    telemetry_tasks: JoinSet<()>,
    telemetry_shutdown: CancellationToken,
    stop_accepting: CancellationToken,
    force_shutdown: CancellationToken,
}

impl Portal {
    pub async fn run(self) -> Result<()> {
        self.inner.telemetry.set_lifecycle(
            LifeState::Starting.to_string(),
            LifeReason::Startup.to_string(),
        );

        let mut signals = match ShutdownSignals::new()
            .context("portal::run: failed to install shutdown signal handlers")
        {
            Ok(signals) => signals,
            Err(error) => return self.start_failed(error),
        };
        let endpoints = match self.listen_endpoints() {
            Ok(endpoints) => endpoints,
            Err(error) => return self.start_failed(error),
        };
        let tcp_listeners = match self.listen_tcp_listeners() {
            Ok(listeners) => listeners,
            Err(error) => return self.start_failed(error),
        };
        for endpoint in &endpoints {
            if let Ok(address) = endpoint.local_addr() {
                self.inner
                    .logger
                    .info(format_args!("portal::run: listening on QUIC/UDP {address}"));
            }
        }
        for listener in &tcp_listeners {
            if let Ok(address) = listener.local_addr() {
                self.inner
                    .logger
                    .info(format_args!("portal::run: listening on TLS/TCP {address}"));
            }
        }
        let telemetry_shutdown = CancellationToken::new();
        let mut telemetry_tasks: JoinSet<()> = JoinSet::new();
        match TelemetryServer::bind(self.inner.telemetry.clone()) {
            Ok(server) => {
                telemetry_tasks.spawn(server.run(telemetry_shutdown.clone()));
                telemetry_tasks.spawn(event::telemetry_loop(
                    self.inner.clone(),
                    telemetry_shutdown.clone(),
                ));
            }
            Err(_) => self.inner.logger.warn(format_args!(
                "portal::run: LOCAL_IPC_UNAVAILABLE; continuing without telemetry"
            )),
        }

        self.log_info("starting");
        let stop_accepting = CancellationToken::new();
        let force_shutdown = CancellationToken::new();

        let mut quic_listeners = JoinSet::new();
        for endpoint in endpoints.iter().cloned() {
            let portal = self.inner.clone();
            let stop_accepting = stop_accepting.clone();
            let force_shutdown = force_shutdown.clone();
            quic_listeners.spawn(async move {
                accept_endpoint_loop(portal, endpoint, stop_accepting, force_shutdown).await;
            });
        }

        let mut tcp_listener_tasks = JoinSet::new();
        for listener in tcp_listeners {
            let portal = self.inner.clone();
            let stop_accepting = stop_accepting.clone();
            let force_shutdown = force_shutdown.clone();
            tcp_listener_tasks.spawn(async move {
                accept_tcp_loop(portal, listener, stop_accepting, force_shutdown).await;
            });
        }

        self.inner.telemetry.set_lifecycle(
            LifeState::Ready.to_string(),
            LifeReason::Listening.to_string(),
        );
        let trigger = tokio::select! {
            signal = signals.recv() => match signal {
                Ok(reason) => ShutdownTrigger { reason, failure: None },
                Err(error) => ShutdownTrigger {
                    reason: LifeReason::SigInt,
                    failure: Some(error.context("portal::run: shutdown signal stream failed")),
                },
            },
            result = quic_listeners.join_next(), if !quic_listeners.is_empty() => {
                ShutdownTrigger {
                    reason: LifeReason::QuicListenerExit,
                    failure: Some(listener_exit_error("QUIC", result)),
                }
            },
            result = tcp_listener_tasks.join_next(), if !tcp_listener_tasks.is_empty() => {
                ShutdownTrigger {
                    reason: LifeReason::TcpListenerExit,
                    failure: Some(listener_exit_error("TCP", result)),
                }
            },
        };

        RunningPortal {
            portal: self,
            signals,
            endpoints,
            quic_listeners,
            tcp_listener_tasks,
            telemetry_tasks,
            telemetry_shutdown,
            stop_accepting,
            force_shutdown,
        }
        .shutdown(trigger)
        .await
    }

    fn start_failed(&self, error: anyhow::Error) -> Result<()> {
        self.inner.telemetry.set_lifecycle(
            LifeState::Stopped.to_string(),
            LifeReason::StartFailed.to_string(),
        );
        self.inner.logger.flush();
        Err(error)
    }

    fn log_info(&self, prefix: &str) {
        self.inner.logger.info(format_args!(
            "portal::run: {prefix}: {}",
            self.effective_url()
        ));
    }
}

fn listener_exit_error(
    name: &str,
    result: Option<std::result::Result<(), tokio::task::JoinError>>,
) -> anyhow::Error {
    match result {
        Some(Ok(())) => anyhow::anyhow!("portal::run: {name} listener exited unexpectedly"),
        Some(Err(error)) => {
            anyhow::anyhow!("portal::run: {name} listener task failed: {error}")
        }
        None => anyhow::anyhow!("portal::run: {name} listener set became empty unexpectedly"),
    }
}

#[cfg(test)]
#[path = "../tests/portal/runtime.rs"]
mod family_error_tests;
