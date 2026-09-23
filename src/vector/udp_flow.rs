// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

//! Target-scoped UDP flow setup and packet transport.

mod framing;
mod setup;

use framing::UotReadState;
pub(crate) use setup::open_udp;

use std::sync::Arc;
use std::sync::atomic::Ordering;

use anyhow::{Context, Result, bail};
use bytes::Bytes;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;
use tokio::time::timeout;

use crate::common::{UdpDatagramSend, handshake_timeout};
use crate::protocol::{Carrier, FlowHeader, FlowKind, FlowRole, Target, write_udp_packet};

use super::PortalClient;
use super::flow::{
    BoxReader, BoxWriter, OpenFlowError, PhysicalLane, SessionGuard, prepare_lanes,
    prepare_with_fallback, read_ready, write_header, write_open_request,
};
use super::flow_id::FlowLease;
use super::route::{ResolvedRoute, plan_route};
use super::session::{QueuedDatagram, QuicSession};
pub(crate) struct UdpTunnel {
    flow_id: u32,
    quic: Option<Arc<QuicSession>>,
    pub(super) uplink: Carrier,
    pub(super) downlink: Carrier,
    sender: UdpTunnelSender,
    receiver: UdpTunnelReceiver,
    _lanes: Vec<PhysicalLane>,
    _lease: Option<FlowLease>,
    _session: Option<SessionGuard>,
}

impl UdpTunnel {
    pub(crate) fn carriers(&self) -> (Carrier, Carrier) {
        (self.uplink, self.downlink)
    }
    pub(crate) async fn send(&mut self, payload: &[u8]) -> Result<bool> {
        self.sender.send(payload).await
    }

    pub(crate) async fn recv_into(
        &mut self,
        payload: &mut Vec<u8>,
    ) -> Result<Option<ReceivedUdpPacket>> {
        self.receiver.recv_into(payload).await
    }

    pub(crate) fn split_mut(&mut self) -> (&mut UdpTunnelSender, &mut UdpTunnelReceiver) {
        (&mut self.sender, &mut self.receiver)
    }

    pub(crate) async fn close(&mut self) {
        self.sender.close().await;
        if let Some(quic) = &self.quic {
            quic.close_udp(self.flow_id);
        }
    }
}

pub(crate) struct UdpTunnelSender {
    flow_id: u32,
    writer: Option<BoxWriter>,
    quic: Option<Arc<QuicSession>>,
    packet_id: u32,
    uplink: Carrier,
    client: Arc<PortalClient>,
}

impl UdpTunnelSender {
    pub(crate) async fn send(&mut self, payload: &[u8]) -> Result<bool> {
        let delivered = if let Some(writer) = &mut self.writer {
            write_udp_packet(writer, payload).await?;
            true
        } else if let Some(quic) = &self.quic {
            quic_datagram_delivered(
                quic.send_udp(self.flow_id, &mut self.packet_id, payload)
                    .await?,
            )
        } else {
            bail!("vector::udp_flow::UdpTunnel::send: no uplink carrier");
        };
        if !delivered {
            return Ok(false);
        }
        if self.client.account_stats {
            self.client
                .stats
                .udp_rx
                .fetch_add(payload.len() as u64, Ordering::Relaxed);
            client_carrier_counter(&self.client, self.uplink, true)
                .fetch_add(payload.len() as u64, Ordering::Relaxed);
        }
        Ok(true)
    }

    async fn close(&mut self) {
        if let Some(writer) = &mut self.writer {
            let _ = timeout(handshake_timeout(), writer.shutdown()).await;
        }
    }
}

pub(crate) struct UdpTunnelReceiver {
    reader: Option<BoxReader>,
    down_datagrams: Option<mpsc::Receiver<QueuedDatagram>>,
    uot_read: UotReadState,
    downlink: Carrier,
    client: Arc<PortalClient>,
}

impl UdpTunnelReceiver {
    pub(crate) async fn recv_into(
        &mut self,
        payload: &mut Vec<u8>,
    ) -> Result<Option<ReceivedUdpPacket>> {
        let packet = if let Some(reader) = &mut self.reader {
            let Some(size) = self.uot_read.read_packet(reader, payload).await? else {
                return Ok(None);
            };
            ReceivedUdpPacket::Buffered(size)
        } else if let Some(receiver) = &mut self.down_datagrams {
            let Some(packet) = receiver.recv().await else {
                return Ok(None);
            };
            ReceivedUdpPacket::Owned(packet.payload)
        } else {
            bail!("vector::udp_flow::UdpTunnel::recv: no downlink carrier");
        };
        let size = packet.len();
        if self.client.account_stats {
            self.client
                .stats
                .udp_tx
                .fetch_add(size as u64, Ordering::Relaxed);
            client_carrier_counter(&self.client, self.downlink, false)
                .fetch_add(size as u64, Ordering::Relaxed);
        }
        Ok(Some(packet))
    }
}

fn quic_datagram_delivered(outcome: UdpDatagramSend) -> bool {
    outcome == UdpDatagramSend::Sent
}

pub(crate) enum ReceivedUdpPacket {
    Buffered(usize),
    Owned(Bytes),
}

impl ReceivedUdpPacket {
    pub(crate) fn len(&self) -> usize {
        match self {
            Self::Buffered(size) => *size,
            Self::Owned(payload) => payload.len(),
        }
    }

    pub(crate) fn payload<'a>(&'a self, buffered: &'a [u8]) -> &'a [u8] {
        match self {
            Self::Buffered(size) => &buffered[..*size],
            Self::Owned(payload) => payload,
        }
    }
}

impl Drop for UdpTunnel {
    fn drop(&mut self) {
        if let Some(quic) = &self.quic {
            quic.remove_udp(self.flow_id);
        }
    }
}

fn client_carrier_counter(
    client: &PortalClient,
    carrier: Carrier,
    uplink: bool,
) -> &std::sync::atomic::AtomicU64 {
    match (carrier, uplink) {
        (Carrier::TlsTcp, true) => &client.stats.up_tcp,
        (Carrier::Quic, true) => &client.stats.up_udp,
        (Carrier::TlsTcp, false) => &client.stats.down_tcp,
        (Carrier::Quic, false) => &client.stats.down_udp,
    }
}

#[cfg(test)]
#[path = "../tests/vector/udp_flow.rs"]
mod tests;
