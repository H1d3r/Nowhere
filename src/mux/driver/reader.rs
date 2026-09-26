// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

//! Mux frame reception and logical-flow dispatch.

use std::io;
use std::sync::Arc;

use bytes::Bytes;
use tokio::io::{AsyncRead, AsyncReadExt};

use super::{closed, frame_charge, invalid};
use crate::mux::wire::{FrameHeader, FrameKind, HEADER_LEN, decode_header};
use crate::mux::{Inbound, ReceiveTarget, Shared};

pub(in crate::mux) async fn run_reader<R: AsyncRead + Unpin>(mut reader: R, shared: Arc<Shared>) {
    let operation = async {
        let mut data_frames = 0_u8;
        loop {
            if shared.closed.load(std::sync::atomic::Ordering::Acquire) {
                return Ok(());
            }
            let mut encoded = [0; HEADER_LEN];
            tokio::select! {
                _ = shared.closed_notify.cancelled() => return Ok(()),
                result = reader.read_exact(&mut encoded) => { result?; }
            }
            let header = decode_header(&encoded).map_err(invalid)?;
            let payload_len = match header.kind {
                FrameKind::Data => header.value as usize,
                FrameKind::Open | FrameKind::Window | FrameKind::Fin | FrameKind::Reset => 0,
            };
            let mut payload = vec![0; payload_len];
            if payload_len != 0 {
                tokio::select! {
                    _ = shared.closed_notify.cancelled() => return Ok(()),
                    result = reader.read_exact(&mut payload) => { result?; }
                }
            }
            match header.kind {
                FrameKind::Open => receive_open(&shared, header).await?,
                FrameKind::Data => receive_data(&shared, header, Bytes::from(payload)).await?,
                FrameKind::Window => receive_window(&shared, header)?,
                FrameKind::Fin | FrameKind::Reset => receive_close(&shared, header).await,
            }
            if payload_len != 0 {
                data_frames = data_frames.wrapping_add(1);
                if data_frames == 32 {
                    data_frames = 0;
                    tokio::task::yield_now().await;
                }
            }
        }
    };
    let result: io::Result<()> = tokio::select! {
        biased;
        _ = shared.closed_notify.cancelled() => return,
        result = operation => result,
    };
    if result.is_err() {
        shared.close();
    }
}

async fn receive_open(shared: &Arc<Shared>, header: FrameHeader) -> io::Result<()> {
    let stream = shared.insert_flow(header.flow_id, true)?;
    let extra_credit = header.value as usize;
    if extra_credit != 0 {
        let credit = shared.send_credit(header.flow_id)?;
        if credit.available_permits().saturating_add(extra_credit)
            > crate::mux::credit_units(crate::mux::MAX_STREAM_WINDOW_BYTES)
        {
            return Err(invalid("stream window overflow"));
        }
        credit.add_permits(extra_credit);
    }
    shared.incoming_tx.try_send(stream).map_err(|_| closed())
}

async fn receive_data(shared: &Arc<Shared>, header: FrameHeader, payload: Bytes) -> io::Result<()> {
    let charge = frame_charge(payload.len());
    match shared.admit_receive(header.flow_id, charge)? {
        ReceiveTarget::Deliver {
            inbound,
            generation,
        } => {
            if inbound.send(Inbound::Data { payload, charge }).is_err() {
                shared.release_receive(header.flow_id, &generation, charge);
            }
        }
        ReceiveTarget::Discard => {
            shared.release_connection_receive(charge);
        }
    }
    Ok(())
}

async fn receive_close(shared: &Shared, header: FrameHeader) {
    if header.kind == FrameKind::Reset {
        if let Some(flow) = shared.remove_flow(header.flow_id) {
            let _ = flow.inbound.send(Inbound::Reset);
        }
        return;
    }
    let (inbound, removed) = {
        let mut flows = shared.flows.lock().expect("mux flow lock");
        let mut inbound = None;
        let mut removed = None;
        if let Some(flow) = flows.get_mut(&header.flow_id)
            && !flow.remote_fin
        {
            if flow.local_parts == 0 && flow.local_fin_sent {
                removed = flows.remove(&header.flow_id);
            } else {
                flow.remote_fin = true;
                inbound = Some(flow.inbound.clone());
            }
        }
        shared
            .active_streams_tx
            .send_replace(crate::mux::active_flow_count(&flows));
        (inbound, removed)
    };
    if let Some(flow) = removed {
        flow.send_credit.close();
        flow.send_slot.close();
    }
    if let Some(inbound) = inbound {
        let _ = inbound.send(Inbound::Fin);
    }
}

fn receive_window(shared: &Shared, header: FrameHeader) -> io::Result<()> {
    let credit = header.value as usize;
    if header.flow_id == 0 {
        if shared
            .connection_send_credit
            .available_permits()
            .saturating_add(credit)
            > crate::mux::credit_units(crate::mux::MAX_CONNECTION_WINDOW_BYTES)
        {
            return Err(invalid("connection window overflow"));
        }
        shared.connection_send_credit.add_permits(credit);
        shared.connection_send_peak.fetch_max(
            shared.connection_send_credit.available_permits(),
            std::sync::atomic::Ordering::Relaxed,
        );
        return Ok(());
    }
    let mut flows = shared.flows.lock().expect("mux flow lock");
    let Some(flow) = flows.get_mut(&header.flow_id) else {
        return Ok(());
    };
    if flow.send_credit.available_permits().saturating_add(credit)
        > crate::mux::credit_units(crate::mux::MAX_STREAM_WINDOW_BYTES)
    {
        return Err(invalid("stream window overflow"));
    }
    flow.send_credit.add_permits(credit);
    Ok(())
}
