// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

//! Mux wire validation and carrier shutdown tests.

use super::*;

async fn assert_raw_frame_closes_carrier(frame: &[u8]) {
    let (left, mut peer) = tokio::io::duplex(1 << 20);
    let (handle, _incoming) = MuxHandle::start(left, MuxConfig::default()).unwrap();
    peer.write_all(frame).await.unwrap();
    tokio::time::timeout(Duration::from_secs(1), handle.closed())
        .await
        .expect("invalid frame must close carrier");
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
