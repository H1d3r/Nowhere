// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

use super::*;

#[tokio::test]
async fn udp_permit_is_shared_by_quic_and_uot_and_released_by_cancel() {
    let registry = registry(1, Duration::from_secs(60));
    let stats = Arc::new(Stats::default());
    let session_id = [6; SESSION_ID_LEN];
    let tcp_guard = registry.register_tcp_link(session_id, stats.clone());
    let quic_guard = registry
        .register_quic_link(
            session_id,
            stats,
            tokio_util::sync::CancellationToken::new(),
        )
        .await;

    let (_datagram_tx, datagram_rx) = mpsc::channel(1);
    assert!(
        registry
            .submit_udp(
                session_id,
                header(
                    FlowRole::Open,
                    13,
                    FlowKind::Udp,
                    Carrier::Quic,
                    Carrier::TlsTcp,
                ),
                Some(target("target.test:53")),
                quic_half("udp-up", quic_guard.quic_generation()),
                UdpHalf::Uplink {
                    uplink: UdpUp::Quic(QuicUdpReceiver::new_without_barrier(
                        datagram_rx,
                        Arc::new(AtomicBool::new(false)),
                        || {},
                    )),
                },
            )
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(available_udp_permits(&registry, session_id), 0);

    let (rejected_uplink, _rejected_uplink_peer) = tokio::io::duplex(64);
    let (rejected_downlink, mut rejected_peer) = tokio::io::duplex(64);
    let rejected = registry
        .submit_udp(
            session_id,
            header(
                FlowRole::Duplex,
                14,
                FlowKind::Udp,
                Carrier::TlsTcp,
                Carrier::TlsTcp,
            ),
            Some(target("target.test:53")),
            tcp_half("uot-limited"),
            UdpHalf::Duplex {
                uplink: UdpUp::TlsTcp(Box::pin(rejected_uplink)),
                downlink: UdpDown::TlsTcp {
                    writer: Box::pin(rejected_downlink),
                    liveness: None,
                },
            },
        )
        .await
        .unwrap_pairing_error();
    assert_eq!(rejected.code(), FlowErrorCode::FlowLimit);
    assert_eq!(
        read_flow_result(&mut rejected_peer).await.unwrap(),
        FlowResult::Reject(FlowErrorCode::FlowLimit)
    );

    registry.cancel_udp(session_id, 13).await;
    assert_eq!(available_udp_permits(&registry, session_id), 1);

    let (uot_uplink, _uot_uplink_peer) = tokio::io::duplex(64);
    let (uot_downlink, _uot_downlink_peer) = tokio::io::duplex(64);
    let paired = registry
        .submit_udp(
            session_id,
            header(
                FlowRole::Duplex,
                14,
                FlowKind::Udp,
                Carrier::TlsTcp,
                Carrier::TlsTcp,
            ),
            Some(target("target.test:53")),
            tcp_half("uot-accepted"),
            UdpHalf::Duplex {
                uplink: UdpUp::TlsTcp(Box::pin(uot_uplink)),
                downlink: UdpDown::TlsTcp {
                    writer: Box::pin(uot_downlink),
                    liveness: None,
                },
            },
        )
        .await
        .unwrap()
        .expect("released permit should admit UoT flow");
    assert_eq!(available_udp_permits(&registry, session_id), 0);
    drop(paired);
    assert_eq!(available_udp_permits(&registry, session_id), 1);

    drop(quic_guard);
    drop(tcp_guard);
}
