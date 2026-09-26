// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

//! Portal TCP stream wrappers for direct, SOCKS5, and native upstreams.

use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

use crate::common::{LatencyGuard, OutboundTcpStream};
use crate::vector::{BoxReader, BoxWriter, TcpTunnel, TcpTunnelGuard};

pub(in crate::portal) enum PortalTcpStream {
    Network(OutboundTcpStream),
    Portal(TcpTunnel),
}

impl PortalTcpStream {
    pub(in crate::portal) fn local_label(&self) -> String {
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

    pub(in crate::portal) fn into_parts(
        self,
    ) -> (PortalTcpReader, PortalTcpWriter, PortalTcpGuard) {
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

pub(in crate::portal) enum PortalTcpReader {
    Network(tokio::net::tcp::OwnedReadHalf),
    Portal(BoxReader),
}

pub(in crate::portal) enum PortalTcpWriter {
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

pub(in crate::portal) struct PortalTcpGuard {
    _resource: PortalTcpResource,
}

enum PortalTcpResource {
    Network { _latency: Option<LatencyGuard> },
    Portal { _guard: TcpTunnelGuard },
}
