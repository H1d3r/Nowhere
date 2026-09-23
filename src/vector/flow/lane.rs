// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

//! Physical carrier acquisition and preparation of TCP flow lanes.

use super::*;

pub(in crate::vector) struct PhysicalLane {
    pub(in crate::vector) reader: Option<BoxReader>,
    pub(in crate::vector) writer: Option<BoxWriter>,
    pending_auth: Option<AuthFrame>,
    pending_quic_auth: bool,
    _link: Option<LinkGuard>,
    _latency: Option<LatencyGuard>,
    pub(in crate::vector) _quic: Option<Arc<QuicSession>>,
}

impl PhysicalLane {
    pub(in crate::vector) fn take_reader(&mut self) -> BoxReader {
        self.reader.take().expect("physical lane reader")
    }

    pub(in crate::vector) fn take_writer(&mut self) -> BoxWriter {
        self.writer.take().expect("physical lane writer")
    }

    pub(in crate::vector) fn take_pending_auth(&mut self) -> Option<AuthFrame> {
        self.pending_auth.take()
    }

    pub(in crate::vector) fn mark_auth_sent(&mut self) {
        self.pending_quic_auth = false;
    }
}

impl Drop for PhysicalLane {
    fn drop(&mut self) {
        if self.pending_quic_auth
            && let Some(session) = &self._quic
        {
            session
                .connection
                .close(quinn::VarInt::from_u32(0), b"authentication abandoned");
        }
    }
}

pub(in crate::vector) async fn open_lane(
    client: Arc<PortalClient>,
    carrier: Carrier,
    flow_id: u32,
) -> Result<PhysicalLane> {
    match carrier {
        Carrier::TlsTcp => {
            let opened = client.tls_manager.open(flow_id).await.map_err(|error| {
                client.telemetry.emit_runtime(RuntimeEvent::new(
                    RuntimeLevel::Warn,
                    RuntimeKind::Carrier,
                    format!("TLS carrier connection failed: {error:#}"),
                ));
                error
            })?;
            match opened {
                OpenedTls::Mux(stream) => {
                    let (reader, writer) = stream.into_split();
                    Ok(PhysicalLane {
                        reader: Some(Box::pin(reader)),
                        writer: Some(Box::pin(writer)),
                        pending_auth: None,
                        pending_quic_auth: false,
                        _link: None,
                        _latency: None,
                        _quic: None,
                    })
                }
                OpenedTls::Dedicated(lane) => {
                    let parts = (*lane).into_parts();
                    Ok(PhysicalLane {
                        reader: Some(Box::pin(parts.reader)),
                        writer: Some(Box::pin(parts.writer)),
                        pending_auth: parts.pending_auth,
                        pending_quic_auth: false,
                        _link: Some(parts.link),
                        _latency: Some(parts.latency),
                        _quic: None,
                    })
                }
            }
        }
        Carrier::Quic => {
            let session = match client.quic.get().await {
                Ok(session) => session,
                Err(error) => {
                    client.telemetry.emit_runtime(RuntimeEvent::new(
                        RuntimeLevel::Warn,
                        RuntimeKind::Reconnect,
                        format!("QUIC carrier connection failed: {error:#}"),
                    ));
                    return Err(error);
                }
            };
            let (writer, reader, pending_auth) = match session.open_bi().await {
                Ok(stream) => stream,
                Err(error) => {
                    client.telemetry.emit_runtime(RuntimeEvent::new(
                        RuntimeLevel::Warn,
                        RuntimeKind::Carrier,
                        format!("QUIC carrier stream open failed: {error:#}"),
                    ));
                    return Err(error);
                }
            };
            let pending_quic_auth = pending_auth.is_some();
            Ok(PhysicalLane {
                reader: Some(Box::pin(reader)),
                writer: Some(Box::pin(writer)),
                pending_auth,
                pending_quic_auth,
                _link: None,
                _latency: None,
                _quic: Some(session),
            })
        }
    }
}

pub(in crate::vector) async fn prepare_lanes(
    client: Arc<PortalClient>,
    route: ResolvedRoute,
    flow_id: u32,
) -> Result<Vec<PhysicalLane>> {
    if !route.split() {
        return Ok(vec![open_lane(client, route.uplink, flow_id).await?]);
    }

    let (uplink, downlink) = tokio::join!(
        open_lane(client.clone(), route.uplink, flow_id),
        open_lane(client, route.downlink, flow_id),
    );
    match (uplink, downlink) {
        (Ok(uplink), Ok(downlink)) => Ok(vec![uplink, downlink]),
        (Err(error), Ok(downlink)) => {
            drop(downlink);
            Err(error)
        }
        (Ok(uplink), Err(error)) => {
            drop(uplink);
            Err(error)
        }
        (Err(uplink), Err(downlink)) => Err(anyhow!(
            "both {} route lanes failed before commit: uplink: {uplink:#}; downlink: {downlink:#}",
            route.label(),
        )),
    }
}
