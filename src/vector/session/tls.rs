// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

use super::*;

pub(in crate::vector) struct TlsManager {
    endpoint: String,
    dialer_ip: String,
    tls: ClientTls,
    auth_key: AuthKey,
    session_id: SessionId,
    stats: Arc<Stats>,
    telemetry: Arc<TelemetryHub>,
    latency: Arc<LatencyTracker>,
    up_mux: Mutex<Vec<MuxHandle>>,
    down_mux: Mutex<Vec<MuxHandle>>,
    up_mux_connect: Mutex<()>,
    down_mux_connect: Mutex<()>,
    mux_enabled: bool,
}

#[derive(Clone, Copy)]
pub(in crate::vector) enum MuxDirection {
    Up,
    Down,
}

pub(in crate::vector) enum OpenedTls {
    Dedicated(Box<TlsLane>),
    Mux(MuxStream),
}

impl TlsManager {
    pub(in crate::vector) fn new(
        config: &PortalClientConfig,
        tls: ClientTls,
        credentials: &Credentials,
        session_id: SessionId,
        signals: ClientSignals,
    ) -> Arc<Self> {
        Arc::new(Self {
            endpoint: config.endpoint(),
            dialer_ip: config.dialer_ip.clone(),
            tls,
            auth_key: credentials.auth_key,
            session_id,
            stats: signals.stats,
            telemetry: signals.telemetry,
            latency: signals.latency,
            up_mux: Mutex::new(Vec::new()),
            down_mux: Mutex::new(Vec::new()),
            up_mux_connect: Mutex::new(()),
            down_mux_connect: Mutex::new(()),
            mux_enabled: config.mux.enabled(),
        })
    }

    pub(in crate::vector) async fn open(
        self: &Arc<Self>,
        flow_id: u32,
        direction: MuxDirection,
    ) -> Result<OpenedTls> {
        if !self.mux_enabled {
            return self
                .connect_lane()
                .await
                .map(Box::new)
                .map(OpenedTls::Dedicated);
        }
        // Serializing stream admission per direction makes the C1 decision
        // exact: the selected shard records its new stream before the next
        // opener observes load.
        let _opening = self.mux_connect(direction).lock().await;
        if let Some(handle) = self.available_mux(direction).await {
            return handle
                .open_stream(flow_id)
                .await
                .map(OpenedTls::Mux)
                .map_err(Into::into);
        }
        let lane = self.connect_lane().await?;
        let TlsLane {
            mut stream,
            pending_auth,
            _link,
            latency,
        } = lane;
        stream
            .write_all(&pending_auth.expect("new TLS carrier has pending auth"))
            .await
            .context("vector::session::TlsManager: failed to authenticate mux carrier")?;
        stream
            .write_u8(crate::common::MUX_MARKER)
            .await
            .context("vector::session::TlsManager: failed to mark mux carrier")?;
        stream.flush().await?;
        let (handle, incoming) = MuxHandle::start(stream, MuxConfig::default())?;
        drop(incoming);
        self.mux(direction).lock().await.push(handle.clone());
        let manager = self.clone();
        let lifetime = handle.clone();
        tokio::spawn(async move {
            manager
                .monitor_mux(direction, lifetime, _link, latency)
                .await;
        });
        handle
            .open_stream(flow_id)
            .await
            .map(OpenedTls::Mux)
            .map_err(Into::into)
    }

    fn mux(&self, direction: MuxDirection) -> &Mutex<Vec<MuxHandle>> {
        match direction {
            MuxDirection::Up => &self.up_mux,
            MuxDirection::Down => &self.down_mux,
        }
    }

    fn mux_connect(&self, direction: MuxDirection) -> &Mutex<()> {
        match direction {
            MuxDirection::Up => &self.up_mux_connect,
            MuxDirection::Down => &self.down_mux_connect,
        }
    }

    async fn available_mux(&self, direction: MuxDirection) -> Option<MuxHandle> {
        let mut muxes = self.mux(direction).lock().await;
        muxes.retain(|handle| !handle.is_closed());
        select_available_mux(&muxes)
    }

    async fn monitor_mux(
        self: Arc<Self>,
        direction: MuxDirection,
        handle: MuxHandle,
        _link: LinkGuard,
        _latency: LatencyGuard,
    ) {
        loop {
            tokio::select! {
                _ = handle.closed() => break,
                idle = handle.idle_for(MUX_IDLE_TIMEOUT) => {
                    if !idle {
                        break;
                    }
                }
            }
            let _opening = self.mux_connect(direction).lock().await;
            if handle.active_streams() != 0 {
                continue;
            }
            self.remove_mux(direction, &handle).await;
            handle.close();
            return;
        }
        let _opening = self.mux_connect(direction).lock().await;
        self.remove_mux(direction, &handle).await;
    }

    async fn remove_mux(&self, direction: MuxDirection, handle: &MuxHandle) {
        self.mux(direction)
            .lock()
            .await
            .retain(|candidate| !candidate.same_carrier(handle));
    }

    async fn connect_lane(&self) -> Result<TlsLane> {
        let (stream, exporter) = self
            .tls
            .connect_tcp(&self.endpoint, &self.dialer_ip)
            .await?;
        let latency = self.latency.register();
        latency.update_tcp(stream.get_ref().0);
        let auth = encode_auth_frame(
            self.auth_key,
            AuthTransport::TlsTcp,
            &exporter,
            self.session_id,
        );
        Ok(TlsLane {
            stream,
            pending_auth: Some(auth),
            _link: LinkGuard::new(self.stats.clone(), self.telemetry.clone(), false),
            latency,
        })
    }
}

fn select_available_mux(muxes: &[MuxHandle]) -> Option<MuxHandle> {
    muxes
        .iter()
        .filter(|handle| !handle.is_closed())
        .min_by_key(|handle| handle.active_streams())
        .filter(|handle| handle.active_streams() < TLS_MUX_FLOWS_PER_SHARD)
        .cloned()
}

#[cfg(test)]
#[path = "../../tests/vector/session/tls.rs"]
mod tests;
