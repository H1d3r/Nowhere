// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

use super::*;

#[tokio::test]
async fn morph_relays_every_fixed_tcp_udp_route() {
    for (up, down) in [
        ("tcp", "tcp"),
        ("tcp", "udp"),
        ("udp", "tcp"),
        ("udp", "udp"),
    ] {
        let target = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let target_address = target.local_addr().unwrap();
        let echo = tokio::spawn(async move {
            let (mut stream, _) = target.accept().await.unwrap();
            let mut request = [0u8; 5];
            stream.read_exact(&mut request).await.unwrap();
            assert_eq!(&request, b"morph");
            stream.write_all(b"works").await.unwrap();
        });
        let runtime = start_runtime_with_morph(up, down, 0, true).await;
        timeout(TEST_TIMEOUT, async {
            let mut socks = TcpStream::connect(runtime.socks).await.unwrap();
            negotiate_socks(&mut socks).await;
            socks
                .write_all(&ip_request(1, target_address))
                .await
                .unwrap();
            read_ipv4_reply(&mut socks).await;
            socks.write_all(b"morph").await.unwrap();
            let mut response = [0u8; 5];
            socks.read_exact(&mut response).await.unwrap();
            assert_eq!(&response, b"works", "up={up} down={down}");
        })
        .await
        .unwrap();
        echo.await.unwrap();
        runtime.stop().await;
    }
}

#[tokio::test]
async fn mux_symmetric_carriers_relay_tcp_and_fragmented_udp() {
    for carrier in ["tcp", "udp"] {
        let tcp_target = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let tcp_address = tcp_target.local_addr().unwrap();
        let tcp_echo = tokio::spawn(async move {
            let (mut stream, _) = tcp_target.accept().await.unwrap();
            let mut ping = [0u8; 4];
            stream.read_exact(&mut ping).await.unwrap();
            stream.write_all(b"pong").await.unwrap();
        });
        let udp_target = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let udp_address = udp_target.local_addr().unwrap();
        // Above QUIC's datagram MTU, below macOS's default UDP socket limit.
        let payload = vec![0x5a; 8 * 1024];
        let echoed = payload.clone();
        let udp_echo = tokio::spawn(async move {
            let mut packet = vec![0u8; 65_507];
            let (length, peer) = udp_target.recv_from(&mut packet).await.unwrap();
            assert_eq!(&packet[..length], echoed);
            udp_target.send_to(&echoed, peer).await.unwrap();
        });
        let runtime = start_runtime(carrier, carrier, 1).await;
        timeout(TEST_TIMEOUT, async {
            let mut tcp = TcpStream::connect(runtime.socks).await.unwrap();
            negotiate_socks(&mut tcp).await;
            tcp.write_all(&ip_request(1, tcp_address)).await.unwrap();
            read_ipv4_reply(&mut tcp).await;
            tcp.write_all(b"ping").await.unwrap();
            let mut pong = [0u8; 4];
            tcp.read_exact(&mut pong).await.unwrap();
            assert_eq!(&pong, b"pong");

            let mut control = TcpStream::connect(runtime.socks).await.unwrap();
            negotiate_socks(&mut control).await;
            control
                .write_all(&ip_request(3, SocketAddr::from(([0, 0, 0, 0], 0))))
                .await
                .unwrap();
            let relay = read_ipv4_reply(&mut control).await;
            let client = UdpSocket::bind("127.0.0.1:0").await.unwrap();
            let mut packet = vec![0, 0, 0];
            packet.extend_from_slice(&ip_request(0, udp_address)[3..]);
            packet.extend_from_slice(&payload);
            client.send_to(&packet, relay).await.unwrap();
            let mut response = vec![0u8; 65_535];
            let (length, _) = client.recv_from(&mut response).await.unwrap();
            assert_eq!(&response[10..length], payload);
        })
        .await
        .unwrap();
        tcp_echo.await.unwrap();
        udp_echo.await.unwrap();
        runtime.stop().await;
    }
}

#[tokio::test]
async fn mux_full_duplex_tcp_exceeds_each_direction_credit_window() {
    full_duplex_exceeds_each_direction_credit_window("tcp").await;
}

#[tokio::test]
async fn quic_full_duplex_tcp_exceeds_each_direction_credit_window() {
    full_duplex_exceeds_each_direction_credit_window("udp").await;
}

async fn full_duplex_exceeds_each_direction_credit_window(carrier: &str) {
    // The throughput profile grants 16 MiB per stream. Cross that boundary in
    // both directions so progress depends on returning Mux credit.
    const DIRECTION_BYTES: usize = 20 * 1024 * 1024;

    {
        let progress = Arc::new(std::array::from_fn::<_, 4, _>(|_| {
            std::sync::atomic::AtomicUsize::new(0)
        }));
        let target = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let target_address = target.local_addr().unwrap();
        let target_progress = progress.clone();
        let target_task = tokio::spawn(async move {
            let (stream, _) = target.accept().await.unwrap();
            stream.set_nodelay(true).unwrap();
            let (mut reader, mut writer) = stream.into_split();
            let upload =
                read_full_duplex_payload(&mut reader, DIRECTION_BYTES, 0xa5, &target_progress[1]);
            let download =
                write_full_duplex_payload(&mut writer, DIRECTION_BYTES, 0x5a, &target_progress[2]);
            tokio::join!(upload, download);
        });
        let runtime = start_runtime(carrier, carrier, 1).await;
        let result = timeout(FULL_DUPLEX_TIMEOUT, async {
            let mut stream = TcpStream::connect(runtime.socks).await.unwrap();
            stream.set_nodelay(true).unwrap();
            negotiate_socks(&mut stream).await;
            stream
                .write_all(&ip_request(1, target_address))
                .await
                .unwrap();
            read_ipv4_reply(&mut stream).await;
            let (mut reader, mut writer) = stream.into_split();
            let upload =
                write_full_duplex_payload(&mut writer, DIRECTION_BYTES, 0xa5, &progress[0]);
            let download =
                read_full_duplex_payload(&mut reader, DIRECTION_BYTES, 0x5a, &progress[3]);
            tokio::join!(upload, download);
        })
        .await;
        if result.is_err() {
            target_task.abort();
            runtime.stop().await;
            panic!(
                "{carrier} full-duplex timeout: client sent={}, target received={}, target sent={}, client received={}, expected={DIRECTION_BYTES}",
                progress[0].load(Ordering::Relaxed),
                progress[1].load(Ordering::Relaxed),
                progress[2].load(Ordering::Relaxed),
                progress[3].load(Ordering::Relaxed)
            );
        }
        target_task.await.unwrap();
        runtime.stop().await;
    }
}

async fn write_full_duplex_payload(
    writer: &mut (impl tokio::io::AsyncWrite + Unpin),
    total: usize,
    byte: u8,
    progress: &std::sync::atomic::AtomicUsize,
) {
    let chunk = [byte; 32 * 1024];
    let mut sent = 0;
    while sent < total {
        let count = writer
            .write(&chunk[..chunk.len().min(total - sent)])
            .await
            .unwrap();
        assert_ne!(count, 0, "full-duplex write made no progress");
        sent += count;
        progress.store(sent, Ordering::Relaxed);
    }
}

async fn read_full_duplex_payload(
    reader: &mut (impl tokio::io::AsyncRead + Unpin),
    total: usize,
    byte: u8,
    progress: &std::sync::atomic::AtomicUsize,
) {
    let mut chunk = [0; 32 * 1024];
    let mut received = 0;
    while received < total {
        let capacity = chunk.len().min(total - received);
        let count = reader.read(&mut chunk[..capacity]).await.unwrap();
        assert_ne!(count, 0, "full-duplex stream ended early");
        assert!(chunk[..count].iter().all(|value| *value == byte));
        received += count;
        progress.store(received, Ordering::Relaxed);
    }
}

#[tokio::test]
async fn mux_hundreds_of_tcp_flows_share_at_most_eight_carriers() {
    const FLOW_COUNT: usize = 300;

    let target = tokio::net::TcpSocket::new_v4().unwrap();
    target.bind("127.0.0.1:0".parse().unwrap()).unwrap();
    let target = target.listen(1024).unwrap();
    let target_address = target.local_addr().unwrap();
    let target_shutdown = CancellationToken::new();
    let target_child_shutdown = target_shutdown.clone();
    let target_task = tokio::spawn(async move {
        let mut connections = Vec::with_capacity(FLOW_COUNT);
        for _ in 0..FLOW_COUNT {
            connections.push(target.accept().await.unwrap().0);
        }
        target_child_shutdown.cancelled().await;
        drop(connections);
    });
    let runtime = start_runtime("tcp", "tcp", 1).await;

    let flows = timeout(TEST_TIMEOUT, async {
        let mut flows = Vec::with_capacity(FLOW_COUNT);
        let mut opening = tokio::task::JoinSet::new();
        for _ in 0..FLOW_COUNT {
            // Exercise concurrent cold admission without overflowing the OS
            // target listener's SYN backlog when the full suite runs in parallel.
            if opening.len() == 32 {
                flows.push(opening.join_next().await.unwrap().unwrap());
            }
            let socks = runtime.socks;
            opening.spawn(async move {
                let mut flow = TcpStream::connect(socks).await.unwrap();
                negotiate_socks(&mut flow).await;
                flow.write_all(&ip_request(1, target_address))
                    .await
                    .unwrap();
                read_ipv4_reply(&mut flow).await;
                flow
            });
        }
        while let Some(flow) = opening.join_next().await {
            flows.push(flow.unwrap());
        }
        flows
    })
    .await
    .unwrap();

    assert_eq!(runtime.portal_stats.link_tcp.load(Ordering::Relaxed), 8);
    assert_eq!(
        runtime.portal_stats.tcp_active.load(Ordering::Relaxed),
        FLOW_COUNT as i32
    );
    drop(flows);
    target_shutdown.cancel();
    target_task.await.unwrap();
    runtime.stop().await;
}
