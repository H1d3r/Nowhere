// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

//! Logical TCP flow setup across every carrier combination.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::task::{Context as TaskContext, Poll};
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;

use crate::common::{
    LatencyGuard, REPLY_CONNECTION_NOT_ALLOWED, REPLY_GENERAL_FAILURE, REPLY_HOST_UNREACHABLE,
    REPLY_NETWORK_UNREACHABLE, REPLY_SUCCEEDED, REPLY_TTL_EXPIRED, SocksAddress,
    flow_setup_timeout, handshake_timeout, mix_fallback_timeout, tcp_read_timeout,
};
use crate::protocol::{
    AUTH_FRAME_LEN, AuthFrame, Carrier, FLOW_HEADER_LEN, FlowHeader, FlowKind, FlowResult,
    FlowRole, SetupResult, TARGET_MAX_ENCODED_LEN, Target, encode_target_into, read_flow_result,
    write_flow_header,
};
use crate::telemetry::{AccessOutcome, AccessSpan, RuntimeEvent, RuntimeKind, RuntimeLevel};

use super::config::CarrierMode;
use super::flow_id::FlowLease;
use super::route::{ResolvedRoute, RoutePlan};
use super::session::{LinkGuard, OpenedTls, QuicSession};
use super::{PortalClient, VectorInner};
mod lane;
mod setup;
mod tcp;

pub(super) use self::lane::{PhysicalLane, prepare_lanes};
pub(super) use self::setup::{prepare_with_fallback, read_ready, write_header, write_open_request};
pub(crate) use self::tcp::{TcpTunnel, TcpTunnelGuard};
pub(super) use self::tcp::{open_tcp, relay_tcp};

pub(crate) type BoxReader = Pin<Box<dyn crate::transport::AsyncReadAny>>;
pub(crate) type BoxWriter = Pin<Box<dyn crate::transport::AsyncWriteAny>>;

pub(super) fn to_target(address: &SocksAddress) -> Result<Target> {
    match address {
        SocksAddress::Ip(address) => Target::ip(*address),
        SocksAddress::Domain(host, port) => Target::domain(host.clone(), *port),
    }
}

pub(super) fn carrier_name(carrier: Carrier) -> &'static str {
    match carrier {
        Carrier::TlsTcp => "TCP",
        Carrier::Quic => "UDP",
    }
}

pub(super) const fn configured_carrier(mode: CarrierMode) -> Option<Carrier> {
    match mode {
        CarrierMode::Tcp => Some(Carrier::TlsTcp),
        CarrierMode::Udp => Some(Carrier::Quic),
        CarrierMode::Mix => None,
    }
}

pub(super) const fn configured_carrier_name(mode: CarrierMode) -> &'static str {
    match mode {
        CarrierMode::Tcp => "TCP",
        CarrierMode::Udp => "UDP",
        CarrierMode::Mix => "MIX",
    }
}

pub(super) fn carrier_counter(
    vector: &VectorInner,
    carrier: Carrier,
    uplink: bool,
) -> &std::sync::atomic::AtomicU64 {
    match (carrier, uplink) {
        (Carrier::TlsTcp, true) => &vector.stats.up_tcp,
        (Carrier::Quic, true) => &vector.stats.up_udp,
        (Carrier::TlsTcp, false) => &vector.stats.down_tcp,
        (Carrier::Quic, false) => &vector.stats.down_udp,
    }
}

#[derive(Debug)]
pub(crate) enum OpenFlowError {
    Setup(SetupResult),
    Transport(anyhow::Error),
    Protocol(anyhow::Error),
}

impl OpenFlowError {
    pub(super) fn socks_reply(&self) -> u8 {
        match self {
            Self::Setup(SetupResult::InvalidRequest | SetupResult::FlowLimit) => {
                REPLY_CONNECTION_NOT_ALLOWED
            }
            Self::Setup(SetupResult::DialFailed) => REPLY_HOST_UNREACHABLE,
            Self::Setup(SetupResult::PairTimeout) => REPLY_TTL_EXPIRED,
            Self::Setup(_) => REPLY_GENERAL_FAILURE,
            Self::Transport(_) => REPLY_NETWORK_UNREACHABLE,
            Self::Protocol(_) => REPLY_GENERAL_FAILURE,
        }
    }

    pub(super) fn access_outcome(&self) -> AccessOutcome {
        match self {
            Self::Setup(SetupResult::InvalidRequest | SetupResult::FlowLimit) => {
                AccessOutcome::Rejected
            }
            Self::Setup(SetupResult::PairTimeout) => AccessOutcome::Timeout,
            Self::Setup(_) | Self::Transport(_) | Self::Protocol(_) => {
                access_error_outcome(&self.to_string())
            }
        }
    }

    pub(crate) fn setup_result(&self) -> Option<SetupResult> {
        match self {
            Self::Setup(result) => Some(*result),
            Self::Transport(_) | Self::Protocol(_) => None,
        }
    }
}

fn access_error_outcome(error: &str) -> AccessOutcome {
    if error.to_ascii_lowercase().contains("timeout") {
        AccessOutcome::Timeout
    } else {
        AccessOutcome::Error
    }
}

impl std::fmt::Display for OpenFlowError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Setup(result) => write!(formatter, "flow setup rejected: {}", result.as_str()),
            Self::Transport(error) | Self::Protocol(error) => {
                if formatter.alternate() {
                    write!(formatter, "{error:#}")
                } else {
                    error.fmt(formatter)
                }
            }
        }
    }
}

pub(super) struct SessionGuard {
    stats: Arc<crate::transport::Stats>,
    udp: bool,
}

impl SessionGuard {
    pub(super) fn new(stats: Arc<crate::transport::Stats>, udp: bool) -> Self {
        Self { stats, udp }
    }
}

impl Drop for SessionGuard {
    fn drop(&mut self) {
        self.stats.done_session(self.udp);
    }
}

#[cfg(test)]
#[path = "../tests/vector/flow.rs"]
mod tests;
