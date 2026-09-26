// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

//! Length-only UDP-over-stream packet framing after setup succeeds.

use anyhow::{Context, Result, bail};
use bytes::Buf;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const UOT_HEADER_LEN: usize = 2;
pub const UOT_PACKET_MAX: usize = u16::MAX as usize;

pub fn encode_udp_packet_header(payload_len: usize) -> Result<[u8; UOT_HEADER_LEN]> {
    if payload_len > UOT_PACKET_MAX {
        bail!("protocol::uot::encode_udp_packet_header: payload too large: {payload_len}");
    }
    Ok((payload_len as u16).to_be_bytes())
}

pub async fn write_udp_packet<W: AsyncWrite + Unpin>(writer: &mut W, payload: &[u8]) -> Result<()> {
    let header = encode_udp_packet_header(payload.len())?;
    let mut frame = Buf::chain(&header[..], payload);
    writer
        .write_all_buf(&mut frame)
        .await
        .context("protocol::uot::write_udp_packet: failed to write packet")
}

pub async fn read_udp_packet_into<R: AsyncRead + Unpin>(
    reader: &mut R,
    payload: &mut Vec<u8>,
) -> Result<Option<usize>> {
    let mut first = [0; 1];
    let count = reader
        .read(&mut first)
        .await
        .context("protocol::uot::read_udp_packet: failed to read packet length")?;
    if count == 0 {
        payload.clear();
        return Ok(None);
    }

    let mut second = [0; 1];
    reader
        .read_exact(&mut second)
        .await
        .context("protocol::uot::read_udp_packet: truncated packet length")?;
    let payload_len = u16::from_be_bytes([first[0], second[0]]) as usize;
    payload.resize(payload_len, 0);
    reader
        .read_exact(payload)
        .await
        .context("protocol::uot::read_udp_packet: truncated packet payload")?;
    Ok(Some(payload_len))
}

#[cfg(test)]
#[path = "../tests/protocol/uot.rs"]
mod tests;
