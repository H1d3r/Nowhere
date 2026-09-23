// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

//! Mux driver wiring and flow-controlled outbound data submission.

use std::io;
use std::sync::Arc;

use super::wire::{FlowId, FrameHeader};
use super::{MuxChunk, Outbound, Shared};

mod reader;
mod writer;

pub(super) use reader::run_reader;
pub(super) use writer::{run_terminals, run_writer};

pub(super) async fn send_data(
    shared: Arc<Shared>,
    flow_id: FlowId,
    payload: MuxChunk,
) -> io::Result<()> {
    let charge = frame_charge(payload.len());
    let (flow_credit, slot) = {
        let flows = shared.flows.lock().expect("mux flow lock");
        let flow = flows.get(&flow_id).ok_or_else(closed)?;
        (flow.send_credit.clone(), flow.send_slot.clone())
    };
    let slot = slot.acquire_owned().await.map_err(|_| closed())?;
    let flow = flow_credit
        .acquire_many_owned(charge as u32)
        .await
        .map_err(|_| closed())?;
    let connection = shared
        .connection_send_credit
        .clone()
        .acquire_many_owned(charge as u32)
        .await
        .map_err(|_| closed())?;
    shared
        .data_tx
        .send(Outbound::Data {
            header: frame_data(flow_id, payload.len())?,
            payload,
            _slot: slot,
        })
        .await
        .map_err(|_| closed())?;
    flow.forget();
    connection.forget();
    Ok(())
}

fn frame_charge(payload: usize) -> usize {
    super::credit_units(payload)
}

pub(super) fn frame_open(flow_id: FlowId, receive_window_bytes: usize) -> io::Result<FrameHeader> {
    let extra = receive_window_bytes.saturating_sub(super::BASE_STREAM_WINDOW_BYTES);
    FrameHeader::open(flow_id, super::credit_units(extra)).map_err(invalid)
}

pub(super) fn frame_data(flow_id: FlowId, length: usize) -> io::Result<FrameHeader> {
    FrameHeader::data(flow_id, length).map_err(invalid)
}

pub(super) fn frame_close(flow_id: FlowId, code: u8) -> io::Result<FrameHeader> {
    FrameHeader::close(flow_id, code).map_err(invalid)
}

fn invalid(error: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error.to_string())
}

pub(super) fn closed() -> io::Error {
    io::Error::new(io::ErrorKind::BrokenPipe, "mux carrier is closed")
}
