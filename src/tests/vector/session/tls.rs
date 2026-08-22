// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

use super::*;

#[tokio::test]
async fn shard_selection_stops_at_twelve_active_flows() {
    let (left, right) = tokio::io::duplex(1 << 20);
    let (handle, _) = MuxHandle::start(left, MuxConfig::default()).unwrap();
    let (_peer, mut incoming) = MuxHandle::start(right, MuxConfig::default()).unwrap();
    let mut streams = Vec::new();
    let mut peers = Vec::new();

    for flow_id in 1..TLS_MUX_FLOWS_PER_SHARD as u32 {
        streams.push(handle.open_stream(flow_id).await.unwrap());
        peers.push(incoming.accept().await.unwrap().unwrap());
    }
    assert!(
        select_available_mux(std::slice::from_ref(&handle))
            .unwrap()
            .same_carrier(&handle)
    );

    streams.push(
        handle
            .open_stream(TLS_MUX_FLOWS_PER_SHARD as u32)
            .await
            .unwrap(),
    );
    peers.push(incoming.accept().await.unwrap().unwrap());
    assert!(select_available_mux(std::slice::from_ref(&handle)).is_none());
}
