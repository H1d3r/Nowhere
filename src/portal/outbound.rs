// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

//! Direct, SOCKS5, or native Portal upstream target establishment.

use std::ops::Range;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use anyhow::anyhow;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::time::Instant;

use crate::common::{LatencyGuard, OutboundDialer, OutboundTcpStream, OutboundUdpSocket};
use crate::protocol::{MAX_PORTAL_HOPS, SetupResult, Target};
use crate::vector::PortalClient;
use crate::vector::flow::{BoxReader, BoxWriter, OpenFlowError, TcpTunnel, TcpTunnelGuard};
use crate::vector::udp_flow::{ReceivedUdpPacket, UdpTunnel, UdpTunnelReceiver, UdpTunnelSender};

pub(super) enum PortalOutbound {
    Network(OutboundDialer),
    Portal(Arc<PortalClient>),
}

impl PortalOutbound {
    pub(super) fn network(dialer: OutboundDialer) -> Self {
        Self::Network(dialer)
    }

    pub(super) fn portal(client: Arc<PortalClient>) -> Self {
        Self::Portal(client)
    }

    pub(super) async fn dial_tcp_target(
        &self,
        target: &Target,
        incoming_hops: u8,
        timeout: Duration,
    ) -> Result<PortalTcpStream, OutboundError> {
        match self {
            Self::Network(dialer) => dialer
                .dial_tcp_target(target, timeout)
                .await
                .map(PortalTcpStream::Network)
                .map_err(OutboundError::transport),
            Self::Portal(client) => {
                let hops = forwarded_hops(incoming_hops)?;
                client
                    .open_tcp(target, hops)
                    .await
                    .map(PortalTcpStream::Portal)
                    .map_err(OutboundError::flow)
            }
        }
    }

    pub(super) async fn dial_udp_target(
        &self,
        target: &Target,
        incoming_hops: u8,
        timeout: Duration,
    ) -> Result<PortalUdpTarget, OutboundError> {
        match self {
            Self::Network(dialer) => dialer
                .dial_udp_target(target, timeout)
                .await
                .map(PortalUdpTarget::Network)
                .map_err(OutboundError::transport),
            Self::Portal(client) => {
                let hops = forwarded_hops(incoming_hops)?;
                client
                    .open_udp(target, hops)
                    .await
                    .map(PortalUdpTarget::Portal)
                    .map_err(OutboundError::flow)
            }
        }
    }

    pub(super) fn dialer_ip(&self) -> &str {
        match self {
            Self::Network(dialer) => dialer.dialer_ip(),
            Self::Portal(client) => client.dialer_ip(),
        }
    }

    pub(super) fn socks_endpoint(&self) -> String {
        match self {
            Self::Network(dialer) => dialer.socks_endpoint(),
            Self::Portal(_) => "none".to_owned(),
        }
    }

    pub(super) fn next_endpoint(&self) -> String {
        match self {
            Self::Network(_) => "none".to_owned(),
            Self::Portal(client) => client.endpoint(),
        }
    }

    pub(super) fn next_transport(&self) -> Option<String> {
        match self {
            Self::Network(_) => None,
            Self::Portal(client) => Some(client.effective_route()),
        }
    }

    pub(super) fn ping_ms(&self) -> u64 {
        match self {
            Self::Network(dialer) => dialer.ping_ms(),
            Self::Portal(client) => client.ping_ms(),
        }
    }

    pub(super) async fn refresh_latency(&self) {
        if let Self::Portal(client) = self {
            client.refresh_latency().await;
        }
    }

    pub(super) async fn close(&self, deadline: Instant) {
        if let Self::Portal(client) = self {
            client.close(deadline).await;
        }
    }
}

fn forwarded_hops(incoming: u8) -> Result<u8, OutboundError> {
    match incoming {
        0 => Ok(MAX_PORTAL_HOPS),
        1 => Err(OutboundError::setup(SetupResult::FlowLimit)),
        value => Ok(value - 1),
    }
}

pub(super) enum PortalTcpStream {
    Network(OutboundTcpStream),
    Portal(TcpTunnel),
}

impl PortalTcpStream {
    pub(super) fn local_label(&self) -> String {
        match self {
            Self::Network(stream) => stream
                .local_addr()
                .map_or_else(|_| "<unknown>".to_owned(), |address| address.to_string()),
            Self::Portal(tunnel) => {
                let (up, down) = tunnel.carriers();
                format!("portal({up:?}/{down:?})")
            }
        }
    }

    pub(super) fn into_parts(self) -> (PortalTcpReader, PortalTcpWriter, PortalTcpGuard) {
        match self {
            Self::Network(stream) => {
                let (reader, writer, latency) = stream.into_split();
                (
                    PortalTcpReader::Network(reader),
                    PortalTcpWriter::Network(writer),
                    PortalTcpGuard {
                        _resource: PortalTcpResource::Network { _latency: latency },
                    },
                )
            }
            Self::Portal(tunnel) => {
                let (reader, writer, guard) = tunnel.into_parts();
                (
                    PortalTcpReader::Portal(reader),
                    PortalTcpWriter::Portal(writer),
                    PortalTcpGuard {
                        _resource: PortalTcpResource::Portal { _guard: guard },
                    },
                )
            }
        }
    }
}

pub(super) enum PortalTcpReader {
    Network(tokio::net::tcp::OwnedReadHalf),
    Portal(BoxReader),
}

pub(super) enum PortalTcpWriter {
    Network(tokio::net::tcp::OwnedWriteHalf),
    Portal(BoxWriter),
}

impl AsyncRead for PortalTcpReader {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        match &mut *self {
            Self::Network(reader) => Pin::new(reader).poll_read(cx, buffer),
            Self::Portal(reader) => reader.as_mut().poll_read(cx, buffer),
        }
    }
}

impl AsyncWrite for PortalTcpWriter {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        match &mut *self {
            Self::Network(writer) => Pin::new(writer).poll_write(cx, buffer),
            Self::Portal(writer) => writer.as_mut().poll_write(cx, buffer),
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match &mut *self {
            Self::Network(writer) => Pin::new(writer).poll_flush(cx),
            Self::Portal(writer) => writer.as_mut().poll_flush(cx),
        }
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match &mut *self {
            Self::Network(writer) => Pin::new(writer).poll_shutdown(cx),
            Self::Portal(writer) => writer.as_mut().poll_shutdown(cx),
        }
    }
}

pub(super) struct PortalTcpGuard {
    _resource: PortalTcpResource,
}

enum PortalTcpResource {
    Network { _latency: Option<LatencyGuard> },
    Portal { _guard: TcpTunnelGuard },
}

pub(super) enum PortalUdpTarget {
    Network(OutboundUdpSocket),
    Portal(UdpTunnel),
}

impl PortalUdpTarget {
    pub(super) fn local_label(&self) -> String {
        match self {
            Self::Network(socket) => socket
                .local_addr()
                .map_or_else(|_| "<unknown>".to_owned(), |address| address.to_string()),
            Self::Portal(tunnel) => {
                let (up, down) = tunnel.carriers();
                format!("portal({up:?}/{down:?})")
            }
        }
    }

    pub(super) fn split_mut(&mut self) -> (PortalUdpSender<'_>, PortalUdpReceiver<'_>) {
        match self {
            Self::Network(socket) => (
                PortalUdpSender::Network(socket),
                PortalUdpReceiver::Network(socket),
            ),
            Self::Portal(tunnel) => {
                let (sender, receiver) = tunnel.split_mut();
                (
                    PortalUdpSender::Portal(sender),
                    PortalUdpReceiver::Portal(receiver),
                )
            }
        }
    }

    pub(super) async fn close(&mut self) {
        if let Self::Portal(tunnel) = self {
            tunnel.close().await;
        }
    }
}

pub(super) enum PortalUdpSender<'a> {
    Network(&'a OutboundUdpSocket),
    Portal(&'a mut UdpTunnelSender),
}

impl PortalUdpSender<'_> {
    pub(super) async fn send(
        &mut self,
        payload: &[u8],
        scratch: &mut Vec<u8>,
    ) -> anyhow::Result<usize> {
        match self {
            Self::Network(socket) => socket.send(payload, scratch).await,
            Self::Portal(sender) => sender
                .send(payload)
                .await
                .map(|sent| if sent { payload.len() } else { 0 }),
        }
    }
}

pub(super) enum PortalUdpReceiver<'a> {
    Network(&'a OutboundUdpSocket),
    Portal(&'a mut UdpTunnelReceiver),
}

impl PortalUdpReceiver<'_> {
    pub(super) async fn recv(
        &mut self,
        buffer: &mut Vec<u8>,
    ) -> anyhow::Result<Option<PortalUdpPacket>> {
        match self {
            Self::Network(socket) => socket
                .recv(buffer)
                .await
                .map(PortalUdpPacket::Network)
                .map(Some),
            Self::Portal(receiver) => receiver
                .recv_into(buffer)
                .await
                .map(|packet| packet.map(PortalUdpPacket::Portal)),
        }
    }
}

pub(super) enum PortalUdpPacket {
    Network(Range<usize>),
    Portal(ReceivedUdpPacket),
}

impl PortalUdpPacket {
    pub(super) fn payload<'a>(&'a self, buffer: &'a [u8]) -> &'a [u8] {
        match self {
            Self::Network(range) => &buffer[range.clone()],
            Self::Portal(packet) => packet.payload(buffer),
        }
    }
}

pub(super) struct OutboundError {
    setup: Option<SetupResult>,
    error: anyhow::Error,
}

impl OutboundError {
    fn setup(result: SetupResult) -> Self {
        Self {
            setup: Some(result),
            error: anyhow!("upstream flow rejected: {}", result.as_str()),
        }
    }

    fn flow(error: OpenFlowError) -> Self {
        Self {
            setup: error.setup_result(),
            error: anyhow!(error.to_string()),
        }
    }

    fn transport(error: impl Into<anyhow::Error>) -> Self {
        Self {
            setup: None,
            error: error.into(),
        }
    }

    pub(super) fn setup_result(&self) -> Option<SetupResult> {
        self.setup
    }
}

impl std::fmt::Display for OutboundError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.error.fmt(formatter)
    }
}

impl std::fmt::Debug for OutboundError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.error.fmt(formatter)
    }
}

impl std::error::Error for OutboundError {}

#[cfg(test)]
#[path = "../tests/portal/outbound.rs"]
mod tests;
