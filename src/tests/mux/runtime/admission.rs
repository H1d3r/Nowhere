// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

//! Mux flow admission and resource limits tests.

use super::*;

#[tokio::test]
async fn idle_carrier_flushes_data_without_a_followup_frame() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let (left, right) = tokio::io::duplex(1024);
        let buffered = tokio::io::BufWriter::with_capacity(4096, left);
        let (client, _) = MuxHandle::start(buffered, MuxConfig::default()).unwrap();
        let (server, mut incoming) = MuxHandle::start(right, MuxConfig::default()).unwrap();
        let mut outgoing = client.open_stream(1).await.unwrap();
        let mut accepted = incoming.accept().await.unwrap().unwrap();
        outgoing.write_all(b"response tail").await.unwrap();
        let mut received = [0; 13];
        accepted.read_exact(&mut received).await.unwrap();
        assert_eq!(&received, b"response tail");
        client.close();
        server.close();
    })
    .await
    .expect("idle Mux writer must flush its buffered carrier");
}

#[tokio::test]
async fn more_than_256_live_streams_transfer_and_half_close() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let (left, right) = tokio::io::duplex(1 << 20);
        let (client, _) = MuxHandle::start(left, MuxConfig::default()).unwrap();
        let (server, mut incoming) = MuxHandle::start(right, MuxConfig::default()).unwrap();
        let mut streams = Vec::new();
        for id in 1..=1024 {
            let outgoing = client.open_stream(id).await.unwrap();
            let accepted = incoming.accept().await.unwrap().unwrap();
            streams.push((outgoing, accepted));
        }
        assert_eq!(client.active_streams(), 1024);
        for (mut outgoing, mut accepted) in streams {
            outgoing.write_all(b"ok").await.unwrap();
            drop(outgoing);
            let mut bytes = Vec::new();
            accepted.read_to_end(&mut bytes).await.unwrap();
            assert_eq!(bytes, b"ok");
        }
        assert_eq!(client.active_streams(), 0);
        client.close();
        server.close();
    })
    .await
    .expect("stream admission must not depend on terminal queue capacity");
}

#[tokio::test]
async fn open_reset_churn_cannot_overflow_pending_incoming_admission() {
    let config = MuxConfig {
        active_stream_limit: 2,
        ..MuxConfig::default()
    };
    let (mut peer, carrier) = tokio::io::duplex(4096);
    let (server, incoming) = MuxHandle::start(carrier, config).unwrap();
    for flow_id in 1..=3 {
        peer.write_all(&encode_header(FrameHeader::open(flow_id, 0).unwrap()).unwrap())
            .await
            .unwrap();
        peer.write_all(&encode_header(FrameHeader::close(flow_id, CLOSE_RESET).unwrap()).unwrap())
            .await
            .unwrap();
    }
    tokio::time::timeout(Duration::from_secs(1), server.closed())
        .await
        .expect("RESET must not bypass pending OPEN admission");
    assert_eq!(incoming.receiver.len(), 2);
    assert_eq!(server.active_streams(), 0);
}

#[tokio::test]
async fn locally_closed_flows_remain_within_the_metadata_limit() {
    let config = MuxConfig {
        active_stream_limit: 2,
        ..MuxConfig::default()
    };
    let (_peer, carrier) = tokio::io::duplex(1);
    let (handle, _incoming) = MuxHandle::start(carrier, config).unwrap();

    for flow_id in 1..=2 {
        drop(handle.prepare_stream(flow_id).unwrap());
    }

    assert!(handle.prepare_stream(3).is_err());
    assert!(!handle.is_closed());
    assert_eq!(handle.active_streams(), 0);
    assert_eq!(handle.shared.flows.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn remote_open_admission_closes_carrier_at_the_metadata_limit() {
    let config = MuxConfig {
        active_stream_limit: 2,
        ..MuxConfig::default()
    };
    let (mut peer, carrier) = tokio::io::duplex(4096);
    let (server, mut incoming) = MuxHandle::start(carrier, config).unwrap();
    let mut streams = Vec::new();

    for flow_id in 1..=2 {
        peer.write_all(&encode_header(FrameHeader::open(flow_id, 0).unwrap()).unwrap())
            .await
            .unwrap();
        let stream = incoming.accept().await.unwrap().unwrap();
        assert_eq!(stream.flow_id(), flow_id);
        streams.push(stream);
    }
    assert_eq!(server.active_streams(), 2);

    peer.write_all(&encode_header(FrameHeader::open(3, 0).unwrap()).unwrap())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(1), server.closed())
        .await
        .expect("OPEN beyond the metadata budget must close the carrier");
    assert_eq!(server.active_streams(), 0);
}

#[tokio::test]
async fn slow_small_packet_reader_does_not_block_other_flows() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let (left, right) = tokio::io::duplex(1 << 20);
        let (client, _) = MuxHandle::start(left, MuxConfig::default()).unwrap();
        let (server, mut incoming) = MuxHandle::start(right, MuxConfig::default()).unwrap();
        let mut slow = client.open_stream(1).await.unwrap();
        let _slow_peer = incoming.accept().await.unwrap().unwrap();
        for _ in 0..1024 {
            slow.write_all(b"x").await.unwrap();
        }
        let mut fast = client.open_stream(2).await.unwrap();
        let mut fast_peer = incoming.accept().await.unwrap().unwrap();
        fast.write_all(b"ok").await.unwrap();
        let mut bytes = [0; 2];
        fast_peer.read_exact(&mut bytes).await.unwrap();
        assert_eq!(&bytes, b"ok");
        client.close();
        server.close();
    })
    .await
    .expect("per-flow queue must not stall the carrier reader");
}

#[test]
fn production_idle_timeout_remains_thirty_seconds() {
    assert_eq!(MUX_IDLE_TIMEOUT, Duration::from_secs(30));
}

#[tokio::test]
async fn abandoned_reader_returns_credit_without_closing_other_streams() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let (left, right) = tokio::io::duplex(1 << 20);
        let (client, _) = MuxHandle::start(left, MuxConfig::default()).unwrap();
        let (server, mut incoming) = MuxHandle::start(right, MuxConfig::default()).unwrap();
        let outgoing = client.open_stream(1).await.unwrap();
        let mut accepted = incoming.accept().await.unwrap().unwrap();
        let (reader, _writer) = outgoing.into_split();
        drop(reader);
        accepted
            .write_all(&vec![0; 2 * MAX_CONNECTION_WINDOW_BYTES])
            .await
            .unwrap();
        let mut other = client.open_stream(2).await.unwrap();
        let mut peer = incoming.accept().await.unwrap().unwrap();
        other.write_all(b"ok").await.unwrap();
        let mut bytes = [0; 2];
        peer.read_exact(&mut bytes).await.unwrap();
        assert_eq!(&bytes, b"ok");
        assert!(!client.is_closed());
        client.close();
        server.close();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn a_blocked_flow_cannot_fill_the_shared_send_queue() {
    let (left, _right) = tokio::io::duplex(1);
    let (handle, _incoming) = MuxHandle::start(left, MuxConfig::default()).unwrap();
    let mut a = handle.open_stream(1).await.unwrap();
    let mut b = handle.open_stream(2).await.unwrap();
    a.write_all(b"queued").await.unwrap();
    let pending = tokio::spawn(async move { a.write_all(b"blocked").await });
    tokio::time::timeout(Duration::from_secs(1), b.write_all(b"other"))
        .await
        .unwrap()
        .unwrap();
    tokio::task::yield_now().await;
    assert!(!pending.is_finished());
    handle.close();
    assert!(
        tokio::time::timeout(Duration::from_secs(1), pending)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );
}
