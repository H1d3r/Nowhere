// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

//! Mux wire validation and carrier shutdown tests.

use super::*;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

struct WriteFailIo;

impl AsyncRead for WriteFailIo {
    fn poll_read(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        _buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Poll::Pending
    }
}

impl AsyncWrite for WriteFailIo {
    fn poll_write(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        _buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Poll::Ready(Err(io::Error::new(
            io::ErrorKind::BrokenPipe,
            "injected write failure",
        )))
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

async fn assert_raw_frame_closes_carrier(frame: &[u8]) {
    let (left, mut peer) = tokio::io::duplex(1 << 20);
    let (handle, _incoming) = MuxHandle::start(left, MuxConfig::default()).unwrap();
    peer.write_all(frame).await.unwrap();
    let reason = tokio::time::timeout(Duration::from_secs(1), handle.close_reason())
        .await
        .expect("invalid frame must close carrier");
    assert_eq!(reason, MuxCloseReason::ProtocolViolation);
}

#[tokio::test]
async fn carrier_eof_and_explicit_close_keep_the_first_reason() {
    let (left, peer) = tokio::io::duplex(1 << 20);
    let (handle, _incoming) = MuxHandle::start(left, MuxConfig::default()).unwrap();
    drop(peer);
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), handle.close_reason())
            .await
            .unwrap(),
        MuxCloseReason::PeerEof
    );

    let (left, _peer) = tokio::io::duplex(1 << 20);
    let (handle, _incoming) = MuxHandle::start(left, MuxConfig::default()).unwrap();
    handle.close_with_reason(MuxCloseReason::IdleTimeout);
    handle.close();
    assert_eq!(handle.close_reason().await, MuxCloseReason::IdleTimeout);
}

#[tokio::test]
async fn writer_failure_closes_carrier_with_writer_reason() {
    let (handle, _incoming) = MuxHandle::start(WriteFailIo, MuxConfig::default()).unwrap();
    let _ = handle.open_stream(1).await;
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), handle.close_reason())
            .await
            .unwrap(),
        MuxCloseReason::WriterFailure
    );
}

#[tokio::test]
async fn dropping_last_handle_closes_carrier_and_driver_tasks() {
    let (left, _peer) = tokio::io::duplex(1 << 20);
    let (handle, incoming) = MuxHandle::start(left, MuxConfig::default()).unwrap();
    let shared = Arc::downgrade(&handle.shared);
    let retained = handle.clone();

    drop(handle);
    assert!(!retained.is_closed());

    drop(retained);
    drop(incoming);
    tokio::time::timeout(Duration::from_secs(1), async {
        while shared.upgrade().is_some() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("Mux driver tasks retained Shared after the last handle dropped");
}

#[tokio::test]
async fn cancelling_blocked_open_releases_prepared_flow() {
    let (carrier, _peer) = tokio::io::duplex(1 << 20);
    let config = MuxConfig {
        outbound_frames: 1,
        ..MuxConfig::default()
    };
    let (handle, _incoming) = MuxHandle::start(carrier, config).unwrap();
    let queue_slot = handle.shared.data_tx.clone().reserve_owned().await.unwrap();
    assert_eq!(handle.shared.data_tx.capacity(), 0);

    let prepared = handle.prepare_stream(1).unwrap();
    let opening = {
        let handle = handle.clone();
        tokio::spawn(async move { handle.open_prepared(prepared).await })
    };
    tokio::task::yield_now().await;
    assert!(!opening.is_finished());

    opening.abort();
    assert!(matches!(opening.await, Err(error) if error.is_cancelled()));
    assert!(!handle.contains_flow(1));
    assert!(!handle.is_closed());

    drop(queue_slot);
    handle.close();
}

#[tokio::test]
async fn cancelled_stale_open_preserves_reused_flow_id() {
    let (carrier, _peer) = tokio::io::duplex(1 << 20);
    let config = MuxConfig {
        outbound_frames: 1,
        ..MuxConfig::default()
    };
    let (handle, _incoming) = MuxHandle::start(carrier, config).unwrap();
    let queue_slot = handle.shared.data_tx.clone().reserve_owned().await.unwrap();
    let stale = handle.prepare_stream(1).unwrap();
    let opening = {
        let handle = handle.clone();
        tokio::spawn(async move { handle.open_prepared(stale).await })
    };
    tokio::task::yield_now().await;
    assert!(!opening.is_finished());

    handle.shared.remove_flow(1).unwrap();
    let replacement = handle.prepare_stream(1).unwrap();
    let replacement_generation = replacement.writer.generation.clone();
    opening.abort();
    assert!(matches!(opening.await, Err(error) if error.is_cancelled()));
    assert!(handle.shared.is_current_flow(1, &replacement_generation));
    assert!(!handle.is_closed());

    drop(queue_slot);
    handle.close();
    drop(replacement);
}

#[tokio::test]
async fn invalid_kind_and_unknown_flow_data_close_carrier() {
    assert_raw_frame_closes_carrier(&[0xff, 0, 0, 0, 0, 0, 1]).await;

    let mut frame = encode_header(FrameHeader::data(99, 1).unwrap())
        .unwrap()
        .to_vec();
    frame.push(0);
    assert_raw_frame_closes_carrier(&frame).await;
}

#[tokio::test]
async fn invalid_prepared_id_does_not_reserve_a_stream_or_close_the_carrier() {
    let (left, _peer) = tokio::io::duplex(1024);
    let (handle, _incoming) = MuxHandle::start(left, MuxConfig::default()).unwrap();
    for id in [0, crate::protocol::MAX_FLOW_ID + 1, u32::MAX] {
        assert!(handle.prepare_stream(id).is_err());
        assert_eq!(handle.active_streams(), 0);
        assert!(!handle.is_closed());
    }
    let _stream = handle.prepare_stream(crate::protocol::MAX_FLOW_ID).unwrap();
    assert_eq!(handle.active_streams(), 1);
    handle.close();
}

#[tokio::test]
async fn duplicate_open_and_credit_overflow_close_carrier() {
    let open = encode_header(FrameHeader::open(7, 0).unwrap()).unwrap();
    let mut duplicate = open.to_vec();
    duplicate.extend_from_slice(&open);
    assert_raw_frame_closes_carrier(&duplicate).await;

    let overflow = encode_header(FrameHeader::window(0, u16::MAX as usize).unwrap()).unwrap();
    assert_raw_frame_closes_carrier(&overflow).await;
}

#[tokio::test]
async fn duplicate_close_and_late_stream_window_are_idempotent() {
    let (left, mut peer) = tokio::io::duplex(1 << 20);
    let (handle, mut incoming) = MuxHandle::start(left, MuxConfig::default()).unwrap();
    peer.write_all(&encode_header(FrameHeader::open(7, 0).unwrap()).unwrap())
        .await
        .unwrap();
    let stream = incoming.accept().await.unwrap().unwrap();
    let reset = encode_header(FrameHeader::close(7, CLOSE_RESET).unwrap()).unwrap();
    peer.write_all(&reset).await.unwrap();
    peer.write_all(&reset).await.unwrap();
    peer.write_all(&encode_header(FrameHeader::window(7, 1).unwrap()).unwrap())
        .await
        .unwrap();
    peer.write_all(&encode_header(FrameHeader::open(9, 0).unwrap()).unwrap())
        .await
        .unwrap();
    let next = tokio::time::timeout(Duration::from_secs(1), incoming.accept())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(next.flow_id(), 9);
    assert!(!handle.is_closed());
    drop(stream);
}

#[tokio::test]
async fn reset_discards_queued_data_and_restores_connection_credit() {
    let (carrier, mut peer) = tokio::io::duplex(1);
    let config = MuxConfig {
        stream_window_bytes: BASE_STREAM_WINDOW_BYTES,
        connection_window_bytes: BASE_CONNECTION_WINDOW_BYTES,
        ..MuxConfig::default()
    };
    let (handle, _incoming) = MuxHandle::start(carrier, config).unwrap();
    let mut stream = handle.open_stream(7).await.unwrap();
    stream.write_all(b"x").await.unwrap();
    assert_eq!(
        handle.shared.connection_send_credit.available_permits(),
        credit_units(BASE_CONNECTION_WINDOW_BYTES) - 1
    );

    peer.write_all(&encode_header(FrameHeader::close(7, CLOSE_RESET).unwrap()).unwrap())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        while handle.contains_flow(7) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    drop(stream);

    let mut frame = [0; super::wire::HEADER_LEN];
    tokio::time::timeout(Duration::from_secs(1), peer.read_exact(&mut frame))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        super::wire::decode_header(&frame).unwrap().kind,
        super::wire::FrameKind::Open
    );
    tokio::time::timeout(Duration::from_secs(1), async {
        while handle.shared.connection_send_credit.available_permits()
            != credit_units(BASE_CONNECTION_WINDOW_BYTES)
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(100), peer.read_u8())
            .await
            .is_err()
    );
    assert_eq!(
        handle.shared.connection_send_credit.available_permits(),
        credit_units(BASE_CONNECTION_WINDOW_BYTES)
    );
    assert!(!handle.is_closed());
}

#[tokio::test]
async fn reused_flow_id_is_isolated_from_stale_stream_handles() {
    let (carrier, mut peer) = tokio::io::duplex(1);
    let config = MuxConfig {
        stream_window_bytes: BASE_STREAM_WINDOW_BYTES,
        connection_window_bytes: BASE_CONNECTION_WINDOW_BYTES,
        ..MuxConfig::default()
    };
    let (handle, _incoming) = MuxHandle::start(carrier, config).unwrap();
    let mut stale = handle.open_stream(7).await.unwrap();
    stale.write_all(b"x").await.unwrap();

    peer.write_all(&encode_header(FrameHeader::close(7, CLOSE_RESET).unwrap()).unwrap())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        while handle.contains_flow(7) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();

    let replacement = handle.open_stream(7).await.unwrap();
    assert!(stale.write_all(b"stale").await.is_err());
    assert!(stale.shutdown().await.is_err());
    drop(stale);
    assert_eq!(handle.active_streams(), 1);

    let mut frames = [0; 2 * super::wire::HEADER_LEN];
    tokio::time::timeout(Duration::from_secs(1), peer.read_exact(&mut frames))
        .await
        .unwrap()
        .unwrap();
    let first = super::wire::decode_header(&frames[..super::wire::HEADER_LEN]).unwrap();
    let second = super::wire::decode_header(&frames[super::wire::HEADER_LEN..]).unwrap();
    assert_eq!(first.kind, super::wire::FrameKind::Open);
    assert_eq!(second.kind, super::wire::FrameKind::Open);
    assert!(
        tokio::time::timeout(Duration::from_millis(100), peer.read_u8())
            .await
            .is_err()
    );
    assert!(handle.contains_flow(7));
    assert!(!handle.is_closed());
    drop(replacement);
}

#[tokio::test]
async fn data_after_fin_closes_carrier() {
    let (left, mut peer) = tokio::io::duplex(1 << 20);
    let (handle, mut incoming) = MuxHandle::start(left, MuxConfig::default()).unwrap();
    peer.write_all(&encode_header(FrameHeader::open(8, 0).unwrap()).unwrap())
        .await
        .unwrap();
    let _stream = incoming.accept().await.unwrap().unwrap();
    peer.write_all(&encode_header(FrameHeader::close(8, CLOSE_FIN).unwrap()).unwrap())
        .await
        .unwrap();
    let mut data = encode_header(FrameHeader::data(8, 1).unwrap())
        .unwrap()
        .to_vec();
    data.push(0);
    peer.write_all(&data).await.unwrap();
    tokio::time::timeout(Duration::from_secs(1), handle.closed())
        .await
        .expect("DATA after FIN must close carrier");
}

#[tokio::test]
async fn data_after_local_fin_is_discarded_until_peer_fin() {
    let config = MuxConfig {
        stream_window_bytes: BASE_STREAM_WINDOW_BYTES,
        connection_window_bytes: BASE_CONNECTION_WINDOW_BYTES,
        ..MuxConfig::default()
    };
    let (carrier, mut peer) = tokio::io::duplex(1 << 20);
    let (handle, mut incoming) = MuxHandle::start(carrier, config).unwrap();
    let stream = handle.open_stream(7).await.unwrap();
    drop(stream);

    let mut outbound = [0; 2 * super::wire::HEADER_LEN];
    tokio::time::timeout(Duration::from_secs(1), peer.read_exact(&mut outbound))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(handle.active_streams(), 0);
    assert!(handle.contains_flow(7));

    let mut data = encode_header(FrameHeader::data(7, 1).unwrap())
        .unwrap()
        .to_vec();
    data.push(0x7a);
    peer.write_all(&data).await.unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        while handle
            .shared
            .pending_connection_credit
            .load(Ordering::Acquire)
            == 0
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(!handle.is_closed());
    assert_eq!(
        handle
            .shared
            .flows
            .lock()
            .unwrap()
            .get(&7)
            .unwrap()
            .receive_credit,
        credit_units(BASE_STREAM_WINDOW_BYTES) - 1
    );

    peer.write_all(&encode_header(FrameHeader::close(7, CLOSE_FIN).unwrap()).unwrap())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        while handle.contains_flow(7) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();

    peer.write_all(&encode_header(FrameHeader::open(9, 0).unwrap()).unwrap())
        .await
        .unwrap();
    let next = incoming.accept().await.unwrap().unwrap();
    assert_eq!(next.flow_id(), 9);
    assert!(!handle.is_closed());
}

#[tokio::test]
async fn idle_deadline_resets_when_a_stream_becomes_active() {
    let (left, right) = tokio::io::duplex(1 << 20);
    let (client, _) = MuxHandle::start(left, MuxConfig::default()).unwrap();
    let (_server, mut incoming) = MuxHandle::start(right, MuxConfig::default()).unwrap();
    let idle = {
        let client = client.clone();
        tokio::spawn(async move { client.idle_for(std::time::Duration::from_millis(80)).await })
    };

    tokio::time::sleep(std::time::Duration::from_millis(40)).await;
    let outgoing = client.open_stream(1).await.unwrap();
    let accepted = incoming.accept().await.unwrap().unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert!(!idle.is_finished());
    drop(outgoing);
    drop(accepted);

    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(200), idle)
            .await
            .unwrap()
            .unwrap()
    );
}
