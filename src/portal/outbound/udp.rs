// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

//! Direct, SOCKS5, or native Portal upstream target establishment.

use std::ops::Range;

use crate::common::OutboundUdpSocket;
use crate::vector::udp_flow::{ReceivedUdpPacket, UdpTunnel, UdpTunnelReceiver, UdpTunnelSender};

pub(in crate::portal) enum PortalUdpTarget {
    Network(OutboundUdpSocket),
    Portal(UdpTunnel),
}

impl PortalUdpTarget {
    pub(in crate::portal) fn local_label(&self) -> String {
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

    pub(in crate::portal) fn split_mut(&mut self) -> (PortalUdpSender<'_>, PortalUdpReceiver<'_>) {
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

    pub(in crate::portal) async fn close(&mut self) {
        if let Self::Portal(tunnel) = self {
            tunnel.close().await;
        }
    }
}

pub(in crate::portal) enum PortalUdpSender<'a> {
    Network(&'a OutboundUdpSocket),
    Portal(&'a mut UdpTunnelSender),
}

impl PortalUdpSender<'_> {
    pub(in crate::portal) async fn send(
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

pub(in crate::portal) enum PortalUdpReceiver<'a> {
    Network(&'a OutboundUdpSocket),
    Portal(&'a mut UdpTunnelReceiver),
}

impl PortalUdpReceiver<'_> {
    pub(in crate::portal) async fn recv(
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

pub(in crate::portal) enum PortalUdpPacket {
    Network(Range<usize>),
    Portal(ReceivedUdpPacket),
}

impl PortalUdpPacket {
    pub(in crate::portal) fn payload<'a>(&'a self, buffer: &'a [u8]) -> &'a [u8] {
        match self {
            Self::Network(range) => &buffer[range.clone()],
            Self::Portal(packet) => packet.payload(buffer),
        }
    }
}
