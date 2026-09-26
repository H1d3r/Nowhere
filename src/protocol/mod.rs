// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

//! Nowhere wire codecs and the shared application protocol identifier.

mod auth;
mod datagram;
mod flow;
mod reassembly;
mod request;
mod result;
mod uot;
mod util;

pub(crate) use auth::{
    AUTH_FRAME_LEN, AuthFrame, AuthKey, AuthTransport, Credentials, TLS_EXPORTER_LEN, TlsExporter,
    encode_auth_frame, read_auth_frame,
};
pub(crate) use datagram::{
    DatagramReassembler, OwnedUdpFragment, OwnedUdpFrame, ReassemblyConfig, ReassemblyOutcome,
    UDP_HEADER_LEN, decode_udp_frame_owned, encode_udp_close, encode_udp_data_header,
    encode_udp_fragments,
};
pub(crate) use flow::{
    Carrier, FLOW_HEADER_LEN, FlowHeader, FlowId, FlowKind, FlowRole, MAX_FLOW_ID, MAX_PORTAL_HOPS,
    SESSION_ID_LEN, SessionId, read_flow_header, write_flow_header,
};
pub(crate) use request::{TARGET_MAX_ENCODED_LEN, Target, encode_target_into, read_request};
pub(crate) use result::{
    FlowErrorCode, FlowResult, SetupResult, read_flow_result, write_flow_result,
};
pub(crate) use uot::{read_udp_packet_into, write_udp_packet};
pub const ALPN: &[u8] = b"nw2";

#[cfg(test)]
#[path = "../tests/protocol/support.rs"]
mod test_support;
#[cfg(test)]
pub(crate) use test_support::*;
