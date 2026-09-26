// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

//! SOCKS5 ingress requests and UDP association routing tests.

use super::udp::{accept_udp_source, try_admit_udp_target, validate_udp_source_request};
use super::*;
use std::time::Duration;
use tokio::io::AsyncWriteExt;

use crate::common::{LogLevel, Logger};
use crate::vector::Vector;
use url::Url;

#[test]
fn source_request_rejects_other_ips_and_domains() {
    let peer: IpAddr = "127.0.0.1".parse().unwrap();
    assert!(
        validate_udp_source_request(&SocksAddress::Ip("127.0.0.2:1234".parse().unwrap()), peer,)
            .is_err()
    );
    assert!(
        validate_udp_source_request(&SocksAddress::Domain("localhost".into(), 1234), peer).is_err()
    );
}

#[test]
fn source_endpoint_locks_first_port() {
    let endpoint = StdMutex::new(None);
    let peer: IpAddr = "127.0.0.1".parse().unwrap();
    assert!(accept_udp_source(
        &endpoint,
        peer,
        "127.0.0.1:1000".parse().unwrap()
    ));
    assert!(accept_udp_source(
        &endpoint,
        peer,
        "127.0.0.1:1000".parse().unwrap()
    ));
    assert!(!accept_udp_source(
        &endpoint,
        peer,
        "127.0.0.1:1001".parse().unwrap()
    ));
}

#[test]
fn socks_client_resource_admission_is_process_wide_and_reusable() {
    let vector = Vector::new(
        Url::parse("vector://secret@127.0.0.1:2000?socks=127.0.0.1:1080").unwrap(),
        Logger::new(LogLevel::None, false),
    )
    .unwrap();
    let limit = vector.inner.socks_client_admission.available_permits();
    let mut held = (0..limit)
        .map(|_| try_admit_client(&vector.inner).unwrap())
        .collect::<Vec<_>>();
    assert!(try_admit_client(&vector.inner).is_none());

    held.pop();
    assert!(try_admit_client(&vector.inner).is_some());
}

#[test]
fn udp_target_resource_admission_is_held_for_the_target_lifetime() {
    let vector = Vector::new(
        Url::parse("vector://secret@127.0.0.1:2000?socks=127.0.0.1:1080").unwrap(),
        Logger::new(LogLevel::None, false),
    )
    .unwrap();
    let admission = &vector.inner.socks_udp_target_admission;
    let limit = admission.available_permits();
    let mut held = (0..limit)
        .map(|_| try_admit_udp_target(admission).unwrap())
        .collect::<Vec<_>>();
    assert!(try_admit_udp_target(admission).is_none());

    held.pop();
    assert!(try_admit_udp_target(admission).is_some());
}

#[tokio::test]
async fn listener_shutdown_interrupts_idle_socks_handshake() {
    let vector = Vector::new(
        Url::parse("vector://secret@127.0.0.1:9?socks=127.0.0.1:1080").unwrap(),
        Logger::new(LogLevel::None, false),
    )
    .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = listener.local_addr().unwrap();
    let shutdown = CancellationToken::new();
    let admission = vector.inner.socks_client_admission.clone();
    let full = admission.available_permits();
    let serving = tokio::spawn(serve_listener(
        vector.inner.clone(),
        listener,
        shutdown.clone(),
    ));
    let _client = TcpStream::connect(endpoint).await.unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        while admission.available_permits() == full {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();

    shutdown.cancel();

    tokio::time::timeout(Duration::from_millis(500), serving)
        .await
        .expect("listener stayed blocked on idle SOCKS handshake")
        .unwrap();
    assert_eq!(admission.available_permits(), full);
}

#[tokio::test]
async fn client_shutdown_interrupts_pending_connect_setup() {
    let portal = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let portal_addr = portal.local_addr().unwrap();
    let (accepted_tx, accepted_rx) = tokio::sync::oneshot::channel();
    let portal_task = tokio::spawn(async move {
        let (stream, _) = portal.accept().await.unwrap();
        let _ = accepted_tx.send(());
        let _stream = stream;
        std::future::pending::<()>().await;
    });
    let vector = Vector::new(
        Url::parse(&format!(
            "vector://secret@{portal_addr}?up=tcp&down=tcp&socks=127.0.0.1:1080"
        ))
        .unwrap(),
        Logger::new(LogLevel::None, false),
    )
    .unwrap();
    let controls = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let connecting = TcpStream::connect(controls.local_addr().unwrap());
    let (accepted, client) = tokio::join!(controls.accept(), connecting);
    let (server, peer) = accepted.unwrap();
    let mut client = client.unwrap();
    let shutdown = CancellationToken::new();
    let handling = tokio::spawn(handle_client(
        vector.inner.clone(),
        server,
        peer,
        shutdown.clone(),
    ));
    client.write_all(&[5, 1, 0]).await.unwrap();
    let mut greeting = [0; 2];
    client.read_exact(&mut greeting).await.unwrap();
    assert_eq!(greeting, [5, 0]);
    client
        .write_all(&[5, COMMAND_CONNECT, 0, 1, 192, 0, 2, 1, 0, 80])
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(1), accepted_rx)
        .await
        .unwrap()
        .unwrap();

    shutdown.cancel();

    tokio::time::timeout(Duration::from_millis(500), handling)
        .await
        .expect("SOCKS client stayed blocked on Portal setup")
        .unwrap()
        .unwrap();
    portal_task.abort();
}

#[tokio::test]
async fn pending_target_setup_does_not_block_control_shutdown() {
    let portal = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let portal_addr = portal.local_addr().unwrap();
    let (accepted_tx, accepted_rx) = tokio::sync::oneshot::channel();
    let portal_task = tokio::spawn(async move {
        let (stream, _) = portal.accept().await.unwrap();
        let _ = accepted_tx.send(());
        let _stream = stream;
        tokio::time::sleep(Duration::from_secs(10)).await;
    });
    let vector = Vector::new(
        Url::parse(&format!(
            "vector://secret@{portal_addr}?up=tcp&down=tcp&socks=127.0.0.1:1080"
        ))
        .unwrap(),
        Logger::new(LogLevel::None, false),
    )
    .unwrap();

    let controls = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let connecting = TcpStream::connect(controls.local_addr().unwrap());
    let (accepted, client) = tokio::join!(controls.accept(), connecting);
    let (server, peer) = accepted.unwrap();
    let mut client = client.unwrap();
    let association = tokio::spawn(run_udp_association(
        vector.inner.clone(),
        server,
        peer,
        SocksAddress::unspecified(),
        CancellationToken::new(),
    ));

    let mut reply = [0u8; 10];
    client.read_exact(&mut reply).await.unwrap();
    assert_eq!(&reply[..4], &[5, REPLY_SUCCEEDED, 0, 1]);
    let udp_endpoint = SocketAddr::from((
        [reply[4], reply[5], reply[6], reply[7]],
        u16::from_be_bytes([reply[8], reply[9]]),
    ));
    let local_udp = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let mut packet = Vec::new();
    encode_udp_packet_into(
        &mut packet,
        &SocksAddress::Ip("192.0.2.1:53".parse().unwrap()),
        b"request",
    )
    .unwrap();
    local_udp.send_to(&packet, udp_endpoint).await.unwrap();
    tokio::time::timeout(Duration::from_secs(1), accepted_rx)
        .await
        .unwrap()
        .unwrap();

    drop(client);
    tokio::time::timeout(Duration::from_millis(500), association)
        .await
        .expect("association stayed blocked on target setup")
        .unwrap()
        .unwrap();
    portal_task.abort();
}
