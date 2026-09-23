// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

//! Mux frame transmission, terminal messages, and receive-window updates.

use std::io::{self, IoSlice};
use std::sync::Arc;

use tokio::io::{AsyncWrite, AsyncWriteExt};
use tokio::sync::mpsc;

use super::{closed, frame_close, invalid};
use crate::mux::wire::{CLOSE_FIN, FlowId, FrameHeader, FrameKind, HEADER_LEN, encode_header};
use crate::mux::{Outbound, Shared};

pub(in crate::mux) async fn run_terminals(
    shared: Arc<Shared>,
    mut terminal_rx: mpsc::Receiver<FlowId>,
) {
    loop {
        if shared.closed.load(std::sync::atomic::Ordering::Acquire) {
            return;
        }
        let flow_id = tokio::select! {
            _ = shared.closed_notify.cancelled() => return,
            flow_id = terminal_rx.recv() => flow_id,
        };
        let Some(flow_id) = flow_id else { return };
        let Ok(header) = frame_close(flow_id, CLOSE_FIN) else {
            continue;
        };
        let sent = tokio::select! {
            _ = shared.closed_notify.cancelled() => return,
            sent = shared.data_tx.send(Outbound::Control(header)) => sent,
        };
        if sent.is_err() {
            return;
        }
    }
}

pub(in crate::mux) async fn run_writer<W: AsyncWrite + Unpin>(
    mut writer: W,
    shared: Arc<Shared>,
    mut data_rx: mpsc::Receiver<Outbound>,
) {
    let mut control = Vec::with_capacity(HEADER_LEN * 64);
    let mut headers = Vec::with_capacity(HEADER_LEN * 256);
    let mut finished_flows = Vec::with_capacity(256);
    let mut pending_item = None;
    let operation = async {
        loop {
            if shared.closed.load(std::sync::atomic::Ordering::Acquire) {
                return Ok(());
            }
            let item = if let Some(item) = pending_item.take() {
                Some(item)
            } else {
                if data_rx.is_empty() {
                    writer.flush().await?;
                }
                tokio::select! {
                    biased;
                    _ = shared.closed_notify.cancelled() => return Ok(()),
                    _ = shared.control_notify.notified() => {
                        write_pending_windows(&mut writer, &shared, &mut control).await?;
                        continue;
                    }
                    item = data_rx.recv() => item,
                }
            };
            let Some(item) = item else { return Ok(()) };
            match item {
                Outbound::Flush(done) => {
                    let result = writer.flush().await;
                    let failed = result.is_err();
                    let _ = done.send(result);
                    if failed {
                        return Err(closed());
                    }
                }
                Outbound::Control(header) => {
                    headers.clear();
                    finished_flows.clear();
                    if header.kind == FrameKind::Fin {
                        finished_flows.push(header.flow_id);
                    }
                    headers.extend_from_slice(&encode_header(header).map_err(invalid)?);
                    while headers.len() < HEADER_LEN * 256 {
                        let Ok(next) = data_rx.try_recv() else { break };
                        match next {
                            Outbound::Control(header) => {
                                if header.kind == FrameKind::Fin {
                                    finished_flows.push(header.flow_id);
                                }
                                headers.extend_from_slice(&encode_header(header).map_err(invalid)?);
                            }
                            next => {
                                pending_item = Some(next);
                                break;
                            }
                        }
                    }
                    writer.write_all(&headers).await?;
                    writer.flush().await?;
                    for flow_id in finished_flows.drain(..) {
                        shared.finish_local_fin(flow_id);
                    }
                }
                Outbound::Data {
                    header,
                    payload,
                    _slot,
                } => {
                    let header = encode_header(header).map_err(invalid)?;
                    write_frame_vectored(&mut writer, &header, payload.as_ref()).await?;
                    drop(_slot);
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

async fn write_frame_vectored<W: AsyncWrite + Unpin>(
    writer: &mut W,
    header: &[u8; HEADER_LEN],
    payload: &[u8],
) -> io::Result<()> {
    let mut header_offset = 0;
    let mut payload_offset = 0;
    while header_offset != header.len() || payload_offset != payload.len() {
        let written = if header_offset != header.len() {
            writer
                .write_vectored(&[
                    IoSlice::new(&header[header_offset..]),
                    IoSlice::new(&payload[payload_offset..]),
                ])
                .await?
        } else {
            writer.write(&payload[payload_offset..]).await?
        };
        if written == 0 {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "failed to write mux frame",
            ));
        }
        let header_remaining = header.len() - header_offset;
        if written <= header_remaining {
            header_offset += written;
        } else {
            header_offset = header.len();
            payload_offset += written - header_remaining;
        }
    }
    Ok(())
}

async fn write_pending_windows<W: AsyncWrite + Unpin>(
    writer: &mut W,
    shared: &Shared,
    encoded: &mut Vec<u8>,
) -> io::Result<()> {
    encoded.clear();
    let connection = shared
        .pending_connection_credit
        .swap(0, std::sync::atomic::Ordering::AcqRel);
    let ready = shared
        .ready_flows
        .lock()
        .expect("mux ready-flow lock")
        .drain(..)
        .collect::<Vec<_>>();
    let flows = {
        let mut flows = shared.flows.lock().expect("mux flow lock");
        ready
            .into_iter()
            .filter_map(|flow_id| {
                let flow = flows.get_mut(&flow_id)?;
                let credit = std::mem::take(&mut flow.pending_receive_credit);
                flow.window_queued = false;
                (credit != 0).then_some((flow_id, credit))
            })
            .collect::<Vec<_>>()
    };
    append_windows(encoded, 0, connection)?;
    for (flow_id, credit) in flows {
        append_windows(encoded, flow_id, credit)?;
    }
    if !encoded.is_empty() {
        writer.write_all(encoded).await?;
        writer.flush().await?;
    }
    Ok(())
}

fn append_windows(encoded: &mut Vec<u8>, flow_id: FlowId, mut credit: usize) -> io::Result<()> {
    while credit != 0 {
        let delta = credit.min(u16::MAX as usize);
        let header = FrameHeader::window(flow_id, delta).map_err(invalid)?;
        encoded.extend_from_slice(&encode_header(header).map_err(invalid)?);
        credit -= delta;
    }
    Ok(())
}
