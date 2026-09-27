// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

//! Flow reset isolation, cancellation, and receive-credit regression tests.

use super::super::wire::{FrameKind, HEADER_LEN, decode_header};
use super::*;

fn config() -> MuxConfig {
    MuxConfig {
        stream_window_bytes: BASE_STREAM_WINDOW_BYTES,
        connection_window_bytes: BASE_CONNECTION_WINDOW_BYTES,
        ..MuxConfig::default()
    }
}

async fn read_frame(peer: &mut tokio::io::DuplexStream) -> (FrameHeader, Vec<u8>) {
    let mut encoded = [0; HEADER_LEN];
    peer.read_exact(&mut encoded).await.unwrap();
    let header = decode_header(&encoded).unwrap();
    let mut payload = vec![
        0;
        if header.kind == FrameKind::Data {
            header.value as usize
        } else {
            0
        }
    ];
    peer.read_exact(&mut payload).await.unwrap();
    (header, payload)
}

async fn expect_frame(peer: &mut tokio::io::DuplexStream, kind: FrameKind, id: FlowId) -> Vec<u8> {
    loop {
        let (header, payload) = read_frame(peer).await;
        if header.kind == FrameKind::Window {
            continue;
        }
        assert_eq!((header.kind, header.flow_id), (kind, id));
        return payload;
    }
}

fn append_data(frames: &mut Vec<u8>, id: FlowId, data: &[u8]) {
    frames.extend_from_slice(&encode_header(FrameHeader::data(id, data.len()).unwrap()).unwrap());
    frames.extend_from_slice(data);
}

#[tokio::test]
async fn flow_errors_preserve_bidirectional_siblings_and_new_opens() {
    tokio::time::timeout(Duration::from_secs(5), async {
        for scenario in 0..4 {
            let (io, mut peer) = tokio::io::duplex(1 << 20);
            let (handle, _incoming) = MuxHandle::start(io, config()).unwrap();
            let mut bad = handle.open_stream(7).await.unwrap();
            expect_frame(&mut peer, FrameKind::Open, 7).await;
            let mut healthy = handle.open_stream(9).await.unwrap();
            expect_frame(&mut peer, FrameKind::Open, 9).await;
            let mut frames = Vec::new();
            match scenario {
                0 => {
                    frames.extend_from_slice(
                        &encode_header(FrameHeader::close(7, CLOSE_FIN).unwrap()).unwrap(),
                    );
                    append_data(&mut frames, 7, b"bad");
                }
                1 => {
                    handle
                        .shared
                        .flows
                        .lock()
                        .unwrap()
                        .get_mut(&7)
                        .unwrap()
                        .receive_credit = 0;
                    append_data(&mut frames, 7, b"bad");
                }
                2 => frames.extend_from_slice(
                    &encode_header(FrameHeader::window(7, u16::MAX as usize).unwrap()).unwrap(),
                ),
                _ => append_data(&mut frames, 99, b"unknown"),
            }
            append_data(&mut frames, 9, b"healthy");
            peer.write_all(&frames).await.unwrap();
            let failed_id = if scenario == 3 { 99 } else { 7 };
            expect_frame(&mut peer, FrameKind::Reset, failed_id).await;
            let mut payload = [0; 7];
            healthy.read_exact(&mut payload).await.unwrap();
            assert_eq!(&payload, b"healthy");
            healthy.write_all(b"reply").await.unwrap();
            assert_eq!(expect_frame(&mut peer, FrameKind::Data, 9).await, b"reply");
            if scenario != 3 {
                assert_eq!(
                    bad.read_u8().await.unwrap_err().kind(),
                    io::ErrorKind::ConnectionReset
                );
            }
            for _ in 0..3 {
                let mut late = Vec::new();
                append_data(&mut late, failed_id, b"late");
                peer.write_all(&late).await.unwrap();
                expect_frame(&mut peer, FrameKind::Reset, failed_id).await;
            }
            let _next = handle.open_stream(11).await.unwrap();
            expect_frame(&mut peer, FrameKind::Open, 11).await;
            assert!(!handle.is_closed());
            handle.close();
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn reset_reader_progresses_while_writer_and_data_queue_are_blocked() {
    tokio::time::timeout(Duration::from_secs(3), async {
        let (io, mut peer) = tokio::io::duplex(1);
        let (handle, mut incoming) = MuxHandle::start(
            io,
            MuxConfig {
                outbound_frames: 1,
                ..config()
            },
        )
        .unwrap();
        let _blocked = handle.open_stream(1).await.unwrap();
        tokio::task::yield_now().await;
        let slot = handle.shared.data_tx.clone().reserve_owned().await.unwrap();
        let mut frames = Vec::new();
        append_data(&mut frames, 99, b"bad");
        frames.extend_from_slice(&encode_header(FrameHeader::open(9, 0).unwrap()).unwrap());
        append_data(&mut frames, 9, b"ok");
        peer.write_all(&frames).await.unwrap();
        let mut healthy = incoming.accept().await.unwrap().unwrap();
        let mut bytes = [0; 2];
        healthy.read_exact(&mut bytes).await.unwrap();
        assert_eq!(&bytes, b"ok");
        assert!(!handle.can_open_flow(99));
        drop(slot);
        expect_frame(&mut peer, FrameKind::Open, 1).await;
        expect_frame(&mut peer, FrameKind::Reset, 99).await;
        handle.close();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn reset_cancels_connection_credit_and_outbound_queue_waiters() {
    tokio::time::timeout(Duration::from_secs(3), async {
        for block_connection in [true, false] {
            let (io, mut peer) = tokio::io::duplex(1 << 20);
            let (handle, _) = MuxHandle::start(
                io,
                MuxConfig {
                    outbound_frames: 1,
                    ..config()
                },
            )
            .unwrap();
            let mut bad = handle.open_stream(7).await.unwrap();
            expect_frame(&mut peer, FrameKind::Open, 7).await;
            let connection = if block_connection {
                Some(
                    handle
                        .shared
                        .connection_send_credit
                        .clone()
                        .acquire_many_owned(credit_units(BASE_CONNECTION_WINDOW_BYTES) as u32)
                        .await
                        .unwrap(),
                )
            } else {
                None
            };
            let queue = if block_connection {
                None
            } else {
                Some(handle.shared.data_tx.clone().reserve_owned().await.unwrap())
            };
            let sending = tokio::spawn(async move { bad.write_all(b"blocked").await });
            tokio::task::yield_now().await;
            assert!(!sending.is_finished());
            peer.write_all(&encode_header(FrameHeader::close(7, CLOSE_RESET).unwrap()).unwrap())
                .await
                .unwrap();
            assert!(sending.await.unwrap().is_err());
            drop(connection);
            drop(queue);
            assert_eq!(
                handle.shared.connection_send_credit.available_permits(),
                credit_units(BASE_CONNECTION_WINDOW_BYTES)
            );
            assert!(!handle.is_closed());
            handle.close();
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn reset_returns_queued_credit_before_reader_runs_and_chunks_release_once() {
    tokio::time::timeout(Duration::from_secs(3), async {
        let (io, mut peer) = tokio::io::duplex(1 << 20);
        let (handle, _) = MuxHandle::start(io, config()).unwrap();
        let stream = handle.open_stream(7).await.unwrap();
        expect_frame(&mut peer, FrameKind::Open, 7).await;
        let (mut reader, _writer) = stream.into_split();
        let mut frames = Vec::new();
        append_data(&mut frames, 7, b"held");
        append_data(&mut frames, 7, b"partial");
        append_data(&mut frames, 7, b"queued");
        peer.write_all(&frames).await.unwrap();
        let held = reader.recv_chunk().await.unwrap().unwrap();
        assert_eq!(reader.read_u8().await.unwrap(), b'p');
        peer.write_all(&encode_header(FrameHeader::window(7, u16::MAX as usize).unwrap()).unwrap())
            .await
            .unwrap();
        expect_frame(&mut peer, FrameKind::Reset, 7).await;
        assert_eq!(
            *handle.shared.connection_receive_credit.lock().unwrap(),
            credit_units(BASE_CONNECTION_WINDOW_BYTES) - 2
        );
        assert_eq!(
            reader.read_u8().await.unwrap_err().kind(),
            io::ErrorKind::ConnectionReset
        );
        assert_eq!(
            *handle.shared.connection_receive_credit.lock().unwrap(),
            credit_units(BASE_CONNECTION_WINDOW_BYTES) - 1
        );
        drop(held);
        drop(reader);
        assert_eq!(
            *handle.shared.connection_receive_credit.lock().unwrap(),
            credit_units(BASE_CONNECTION_WINDOW_BYTES)
        );
        let mut returned = 0;
        while let Ok((header, _)) =
            tokio::time::timeout(Duration::from_millis(20), read_frame(&mut peer)).await
        {
            assert_eq!(header.kind, FrameKind::Window);
            assert_eq!(header.flow_id, 0);
            returned += header.value as usize;
        }
        returned += handle
            .shared
            .pending_connection_credit
            .load(Ordering::Acquire);
        assert_eq!(returned, 3);
        assert!(!handle.is_closed());
        handle.close();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn stale_reset_cannot_remove_replacement_generation() {
    let (io, _peer) = tokio::io::duplex(1 << 20);
    let (handle, _) = MuxHandle::start(io, config()).unwrap();
    let old = handle.prepare_stream(7).unwrap();
    let generation = old.writer.generation.clone();
    handle.shared.remove_flow(7).unwrap();
    let replacement = handle.prepare_stream(7).unwrap();
    assert!(!handle.shared.prepare_reset(7, Some(&generation)).unwrap());
    assert!(!handle.shared.prepare_reset(7, None).unwrap());
    drop(old);
    assert!(
        handle
            .shared
            .is_current_flow(7, &replacement.writer.generation)
    );
    assert!(!handle.is_closed());
    handle.close();
}

#[tokio::test]
async fn healthy_flows_cross_windows_after_sibling_reset() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let (left, right) = tokio::io::duplex(64 * 1024);
        let (client, _) = MuxHandle::start(left, config()).unwrap();
        let (server, mut incoming) = MuxHandle::start(right, config()).unwrap();
        let mut bad = client.open_stream(7).await.unwrap();
        let mut bad_peer = incoming.accept().await.unwrap().unwrap();
        let healthy = client.open_stream(9).await.unwrap();
        let healthy_peer = incoming.accept().await.unwrap().unwrap();
        server
            .shared
            .flows
            .lock()
            .unwrap()
            .get_mut(&7)
            .unwrap()
            .receive_credit = 0;
        bad.write_all(b"bad").await.unwrap();
        assert_eq!(
            bad_peer.read_u8().await.unwrap_err().kind(),
            io::ErrorKind::ConnectionReset
        );
        assert_eq!(
            bad.read_u8().await.unwrap_err().kind(),
            io::ErrorKind::ConnectionReset
        );
        let (mut a_read, mut a_write) = healthy.into_split();
        let (mut b_read, mut b_write) = healthy_peer.into_split();
        let count = 2 * BASE_CONNECTION_WINDOW_BYTES;
        let a = vec![0x61; count];
        let b = vec![0x62; count];
        let mut a_received = vec![0; count];
        let mut b_received = vec![0; count];
        let (sent_a, sent_b, read_a, read_b) = tokio::join!(
            a_write.write_all(&a),
            b_write.write_all(&b),
            a_read.read_exact(&mut a_received),
            b_read.read_exact(&mut b_received)
        );
        sent_a.unwrap();
        sent_b.unwrap();
        read_a.unwrap();
        read_b.unwrap();
        assert_eq!(a_received, b);
        assert_eq!(b_received, a);
        let _next = client.open_stream(11).await.unwrap();
        assert_eq!(incoming.accept().await.unwrap().unwrap().flow_id(), 11);
        assert!(!client.is_closed());
        assert!(!server.is_closed());
        client.close();
        server.close();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn reset_wakes_pending_read_and_flush_without_waiting_for_carrier() {
    tokio::time::timeout(Duration::from_secs(3), async {
        let (io, mut peer) = tokio::io::duplex(1);
        let (handle, _) = MuxHandle::start(io, config()).unwrap();
        let stream = handle.open_stream(7).await.unwrap();
        let (mut reader, mut writer) = stream.into_split();
        let reading = tokio::spawn(async move { reader.read_u8().await });
        let flushing = tokio::spawn(async move { writer.flush().await });
        tokio::task::yield_now().await;
        assert!(!reading.is_finished());
        assert!(!flushing.is_finished());
        peer.write_all(&encode_header(FrameHeader::close(7, CLOSE_RESET).unwrap()).unwrap())
            .await
            .unwrap();
        assert_eq!(
            reading.await.unwrap().unwrap_err().kind(),
            io::ErrorKind::ConnectionReset
        );
        assert!(flushing.await.unwrap().is_err());
        assert!(!handle.is_closed());
        handle.close();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn pending_resets_are_bounded_and_prevent_early_id_reuse() {
    let (io, _peer) = tokio::io::duplex(1);
    let (handle, _) = MuxHandle::start(
        io,
        MuxConfig {
            active_stream_limit: 2,
            ..config()
        },
    )
    .unwrap();
    let stream = handle.prepare_stream(7).unwrap();
    assert!(
        handle
            .shared
            .prepare_reset(7, Some(&stream.writer.generation))
            .unwrap()
    );
    assert!(handle.shared.prepare_reset(9, None).unwrap());
    assert!(!handle.shared.prepare_reset(7, None).unwrap());
    assert!(!handle.can_open_flow(7));
    assert!(handle.prepare_stream(7).is_err());
    assert!(handle.shared.prepare_reset(11, None).is_err());
    assert_eq!(handle.shared.pending_resets.lock().unwrap().len(), 2);
    handle.shared.finish_reset(7);
    let replacement = handle.prepare_stream(7).unwrap();
    drop(stream);
    assert!(
        handle
            .shared
            .is_current_flow(7, &replacement.writer.generation)
    );
    handle.close();
}

#[tokio::test]
async fn malformed_and_truncated_frames_remain_carrier_fatal() {
    tokio::time::timeout(Duration::from_secs(3), async {
        for frame in [
            vec![0xff, 0, 0, 0, 0, 0, 7],
            vec![0x02, 0, 0, 0, 0, 0, 7],
            vec![0x03, 0, 0, 0, 0, 0, 7],
            vec![0x04, 0, 1, 0, 0, 0, 7],
            vec![0x02, 0, 1, 0x40, 0, 0, 7],
            vec![0x02, 0, 1],
            encode_header(FrameHeader::data(7, 1).unwrap())
                .unwrap()
                .to_vec(),
        ] {
            let (io, mut peer) = tokio::io::duplex(1 << 20);
            let (handle, _) = MuxHandle::start(io, config()).unwrap();
            let _healthy = handle.open_stream(9).await.unwrap();
            peer.write_all(&frame).await.unwrap();
            peer.shutdown().await.unwrap();
            handle.closed().await;
            assert_eq!(handle.active_streams(), 0);
        }
    })
    .await
    .unwrap();
}
