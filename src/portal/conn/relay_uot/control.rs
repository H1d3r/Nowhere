// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

use super::*;

async fn send_udp_result(downlink: &mut UdpDown, result: FlowResult) -> anyhow::Result<()> {
    match downlink {
        UdpDown::TlsTcp { writer, .. } => {
            write_flow_result(writer, result).await?;
            if matches!(result, FlowResult::Reject(_)) {
                writer.shutdown().await?;
            }
        }
        UdpDown::Quic { control, .. } => {
            send_quic_control_result(control, result).await?;
        }
    }
    Ok(())
}

pub(super) async fn send_quic_control_result(
    control: &mut crate::portal::pairing::BoxWriter,
    result: FlowResult,
) -> anyhow::Result<()> {
    write_flow_result(control, result).await?;
    control.shutdown().await?;
    Ok(())
}

/// Commits the single setup result. As with TCP, cancellation is sampled only
/// before READY starts so a partially written READY is never followed by a
/// second control result.
pub(super) async fn commit_udp_ready(
    cancel: &tokio_util::sync::CancellationToken,
    ready_gate: &crate::portal::tasks::ReadyGate,
    downlink: &mut UdpDown,
) -> anyhow::Result<bool> {
    if cancel.is_cancelled() {
        send_udp_result_bounded(downlink, FlowResult::Reject(FlowErrorCode::SessionReplaced))
            .await?;
        return Ok(false);
    }
    let Some(_ready_permit) = ready_gate.try_enter() else {
        send_udp_result_bounded(downlink, FlowResult::Reject(FlowErrorCode::FlowLimit)).await?;
        return Ok(false);
    };
    if cancel.is_cancelled() {
        send_udp_result_bounded(downlink, FlowResult::Reject(FlowErrorCode::SessionReplaced))
            .await?;
        return Ok(false);
    }
    send_udp_result_bounded(downlink, FlowResult::Ready).await?;
    Ok(true)
}

pub(super) async fn send_udp_result_bounded(
    downlink: &mut UdpDown,
    result: FlowResult,
) -> anyhow::Result<()> {
    tokio::time::timeout(FLOW_RESULT_TIMEOUT, send_udp_result(downlink, result))
        .await
        .map_err(|_| anyhow::anyhow!("flow result write timeout"))?
}

pub(super) async fn send_paired_udp(
    downlink: &mut UdpDown,
    flow_id: u32,
    packet_id: &mut u32,
    payload: &[u8],
) -> anyhow::Result<UdpDatagramSend> {
    match downlink {
        UdpDown::TlsTcp { writer, .. } => {
            write_udp_packet(writer, payload).await?;
            Ok(UdpDatagramSend::Sent)
        }
        UdpDown::Quic { conn, .. } => send_quic_udp_packet(conn, flow_id, packet_id, payload).await,
    }
}

async fn send_udp_close(downlink: &mut UdpDown, flow_id: u32) -> anyhow::Result<()> {
    match downlink {
        UdpDown::TlsTcp { writer, .. } => {
            writer.shutdown().await?;
        }
        UdpDown::Quic { conn, .. } => {
            conn.send_datagram_wait(Bytes::copy_from_slice(&encode_udp_close(flow_id)?))
                .await?;
        }
    }
    Ok(())
}

pub(super) async fn finish_udp_downlink(
    downlink: &mut UdpDown,
    flow_id: u32,
    frame_incomplete: bool,
) {
    // A cancelled write_all may have emitted only a prefix of a UoT DATA
    // frame. Appending CLOSE would corrupt the stream; each UoT flow owns its
    // connection, so EOF is the only safe termination in that case. QUIC
    // DATAGRAM frames are atomic and can still receive an advisory CLOSE.
    if frame_incomplete && matches!(&*downlink, UdpDown::TlsTcp { .. }) {
        return;
    }
    let _ = tokio::time::timeout(FLOW_CLOSE_TIMEOUT, send_udp_close(downlink, flow_id)).await;
}
