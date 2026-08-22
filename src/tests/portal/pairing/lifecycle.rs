// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

use super::*;

#[tokio::test]
async fn tcp_flow_limit_is_scoped_to_each_authenticated_session() {
    let mut registry = registry(8, Duration::from_secs(30));
    Arc::get_mut(&mut registry).unwrap().max_tcp_flows = 1;
    let stats = Arc::new(Stats::default());
    let first_session = [0x31; SESSION_ID_LEN];
    let second_session = [0x32; SESSION_ID_LEN];
    let _first_guard = registry.register_tcp_link(first_session, stats.clone());
    let _second_guard = registry.register_tcp_link(second_session, stats);

    let (first_io, _) = tokio::io::duplex(64);
    let (first_down, _) = tokio::io::duplex(64);
    let first = registry
        .submit_tcp(
            first_session,
            header(
                FlowRole::Duplex,
                1,
                FlowKind::Tcp,
                Carrier::TlsTcp,
                Carrier::TlsTcp,
            ),
            Some(target("target.test:443")),
            tcp_half("first"),
            Some(Box::pin(first_io)),
            Some(Box::pin(first_down)),
            None,
        )
        .await
        .unwrap()
        .unwrap();

    let (excess_io, _) = tokio::io::duplex(64);
    let (excess_down, mut excess_peer) = tokio::io::duplex(64);
    let error = registry
        .submit_tcp(
            first_session,
            header(
                FlowRole::Duplex,
                2,
                FlowKind::Tcp,
                Carrier::TlsTcp,
                Carrier::TlsTcp,
            ),
            Some(target("target.test:443")),
            tcp_half("excess"),
            Some(Box::pin(excess_io)),
            Some(Box::pin(excess_down)),
            None,
        )
        .await
        .unwrap_pairing_error();
    assert_eq!(error.code(), FlowErrorCode::FlowLimit);
    assert_eq!(
        read_flow_result(&mut excess_peer).await.unwrap(),
        FlowResult::Reject(FlowErrorCode::FlowLimit)
    );

    let (other_io, _) = tokio::io::duplex(64);
    let (other_down, _) = tokio::io::duplex(64);
    let other = registry
        .submit_tcp(
            second_session,
            header(
                FlowRole::Duplex,
                1,
                FlowKind::Tcp,
                Carrier::TlsTcp,
                Carrier::TlsTcp,
            ),
            Some(target("target.test:443")),
            tcp_half("other"),
            Some(Box::pin(other_io)),
            Some(Box::pin(other_down)),
            None,
        )
        .await
        .unwrap();
    assert!(other.is_some());

    drop(first);
    drop(other);
}

#[tokio::test]
async fn drain_rejects_pending_and_new_flows_but_preserves_active_claims() {
    let registry = registry(8, Duration::from_secs(30));
    let stats = Arc::new(Stats::default());
    let session_id = [0x5a; SESSION_ID_LEN];
    let _tcp_guard = registry.register_tcp_link(session_id, stats);

    let (active_up, _active_up_peer) = tokio::io::duplex(64);
    let (active_down, _active_down_peer) = tokio::io::duplex(64);
    let active = registry
        .submit_tcp(
            session_id,
            header(
                FlowRole::Duplex,
                40,
                FlowKind::Tcp,
                Carrier::TlsTcp,
                Carrier::TlsTcp,
            ),
            Some(target("target.test:443")),
            tcp_half("active"),
            Some(Box::pin(active_up)),
            Some(Box::pin(active_down)),
            None,
        )
        .await
        .unwrap()
        .expect("flow should activate before drain");
    let active_cancel = active._flow_lease.cancellation_token();

    let (pending_up, _pending_peer) = tokio::io::duplex(64);
    assert!(
        registry
            .submit_tcp(
                session_id,
                header(
                    FlowRole::Open,
                    41,
                    FlowKind::Tcp,
                    Carrier::TlsTcp,
                    Carrier::TlsTcp,
                ),
                Some(target("target.test:443")),
                tcp_half("pending"),
                Some(Box::pin(pending_up)),
                None,
                None,
            )
            .await
            .unwrap()
            .is_none()
    );

    registry.begin_drain().await;
    assert!(!registry.is_accepting());
    assert!(!active_cancel.is_cancelled());
    assert!(registry.tcp.lock().await.is_empty());

    let (late_down, mut late_peer) = tokio::io::duplex(64);
    let error = registry
        .submit_tcp(
            session_id,
            header(
                FlowRole::Attach,
                41,
                FlowKind::Tcp,
                Carrier::TlsTcp,
                Carrier::TlsTcp,
            ),
            None,
            tcp_half("late"),
            None,
            Some(Box::pin(late_down)),
            None,
        )
        .await
        .unwrap_pairing_error();
    assert_eq!(error.code(), FlowErrorCode::FlowLimit);
    assert_eq!(
        read_flow_result(&mut late_peer).await.unwrap(),
        FlowResult::Reject(FlowErrorCode::FlowLimit)
    );

    let (new_up, _new_up_peer) = tokio::io::duplex(64);
    let (new_down, mut new_peer) = tokio::io::duplex(64);
    let error = registry
        .submit_tcp(
            session_id,
            header(
                FlowRole::Duplex,
                42,
                FlowKind::Tcp,
                Carrier::TlsTcp,
                Carrier::TlsTcp,
            ),
            Some(target("target.test:443")),
            tcp_half("new"),
            Some(Box::pin(new_up)),
            Some(Box::pin(new_down)),
            None,
        )
        .await
        .unwrap_pairing_error();
    assert_eq!(error.code(), FlowErrorCode::FlowLimit);
    assert_eq!(
        read_flow_result(&mut new_peer).await.unwrap(),
        FlowResult::Reject(FlowErrorCode::FlowLimit)
    );

    drop(active);
}

#[tokio::test]
async fn cancel_all_cancels_active_flows_without_waiting_for_pending_writer() {
    let registry = registry(8, Duration::from_secs(60));
    let stats = Arc::new(Stats::default());
    let session_id = [5; SESSION_ID_LEN];
    let tcp_guard = registry.register_tcp_link(session_id, stats.clone());
    let quic_guard = registry
        .register_quic_link(
            session_id,
            stats,
            tokio_util::sync::CancellationToken::new(),
        )
        .await;

    assert!(
        registry
            .submit_tcp(
                session_id,
                header(
                    FlowRole::Attach,
                    11,
                    FlowKind::Tcp,
                    Carrier::TlsTcp,
                    Carrier::Quic,
                ),
                None,
                quic_half("blocked", quic_guard.quic_generation()),
                None,
                Some(Box::pin(PendingWriter)),
                None,
            )
            .await
            .unwrap()
            .is_none()
    );

    let (active_stream, _active_peer) = tokio::io::duplex(64);
    let (active_downlink, _downlink_peer) = tokio::io::duplex(64);
    let active = registry
        .submit_tcp(
            session_id,
            header(
                FlowRole::Duplex,
                12,
                FlowKind::Tcp,
                Carrier::TlsTcp,
                Carrier::TlsTcp,
            ),
            Some(target("target.test:443")),
            tcp_half("active"),
            Some(Box::pin(active_stream)),
            Some(Box::pin(active_downlink)),
            None,
        )
        .await
        .unwrap()
        .expect("duplex flow should activate");
    let active_cancel = active._flow_lease.cancellation_token();

    tokio::time::timeout(Duration::from_millis(500), registry.cancel_all())
        .await
        .expect("cancel_all must not await a blocked network writer");
    assert!(active_cancel.is_cancelled());
    assert!(registry.tcp.lock().await.is_empty());
    assert!(registry.udp.lock().await.is_empty());
    assert_eq!(
        registry
            .claims
            .lock()
            .expect("flow claim registry poisoned")
            .len(),
        1,
        "only the cancelled active lease remains until it is dropped"
    );
    drop(active);
    assert!(
        registry
            .claims
            .lock()
            .expect("flow claim registry poisoned")
            .is_empty()
    );

    drop(quic_guard);
    drop(tcp_guard);
}
