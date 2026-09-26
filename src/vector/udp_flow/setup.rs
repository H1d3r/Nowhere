// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

//! UDP tunnel lane preparation and reliable flow setup.

use super::*;

pub(crate) async fn open_udp(
    client: Arc<PortalClient>,
    target: &Target,
    hops: u8,
) -> std::result::Result<UdpTunnel, OpenFlowError> {
    let lease = client
        .flow_ids
        .allocate()
        .map_err(OpenFlowError::Protocol)?;
    let plan = plan_route(
        client.config.up,
        client.config.down,
        client.route_seed,
        lease.id(),
    );
    let prepare_client = client.clone();
    let (prepared, lease, route) =
        prepare_with_fallback(&client, lease, plan, move |flow_id, route| {
            prepare_udp_attempt(prepare_client.clone(), flow_id, route)
        })
        .await?;
    let flow_id = lease.id();
    let uplink = route.uplink;
    let downlink = route.downlink;
    let split_lanes = route.split();
    let PreparedUdpAttempt {
        mut lanes,
        quic,
        mut down_datagrams,
        mut pending_udp_route,
    } = prepared;
    setup_udp_lanes(&mut lanes, flow_id, route, target, hops).await?;
    if down_datagrams.is_some()
        && let Err(error) = quic
            .as_ref()
            .expect("QUIC downlink has session")
            .activate_udp(flow_id)
    {
        return Err(OpenFlowError::Transport(error));
    }
    if let Some(pending_udp_route) = pending_udp_route.take() {
        pending_udp_route.commit();
    }

    let writer = if uplink == Carrier::TlsTcp {
        Some(lanes[0].take_writer())
    } else {
        None
    };
    let down_index = usize::from(split_lanes);
    let reader = if downlink == Carrier::TlsTcp {
        Some(lanes[down_index].take_reader())
    } else {
        None
    };
    if client.account_stats {
        client.stats.add_session(true);
    }
    Ok(UdpTunnel {
        flow_id,
        uplink,
        downlink,
        sender: UdpTunnelSender {
            flow_id,
            writer,
            quic: quic.clone(),
            packet_id: 1,
            uplink,
            client: client.clone(),
        },
        receiver: UdpTunnelReceiver {
            reader,
            down_datagrams: down_datagrams.take(),
            uot_read: UotReadState::default(),
            downlink,
            client: client.clone(),
        },
        quic,
        _session: client
            .account_stats
            .then(|| SessionGuard::new(client.stats.clone(), true)),
        _lanes: lanes,
        _lease: Some(lease),
    })
}

struct PreparedUdpAttempt {
    lanes: Vec<PhysicalLane>,
    quic: Option<Arc<QuicSession>>,
    down_datagrams: Option<mpsc::Receiver<QueuedDatagram>>,
    pending_udp_route: Option<PendingUdpRoute>,
}

async fn prepare_udp_attempt(
    client: Arc<PortalClient>,
    flow_id: u32,
    route: ResolvedRoute,
) -> Result<PreparedUdpAttempt> {
    let lanes = prepare_lanes(client, route, flow_id).await?;
    let quic = lanes.iter().find_map(|lane| lane._quic.clone());
    let (down_datagrams, pending_udp_route) = if route.downlink == Carrier::Quic {
        let (down_datagrams, pending_udp_route) = quic
            .as_ref()
            .expect("QUIC downlink has session")
            .register_udp(flow_id)?;
        (Some(down_datagrams), Some(pending_udp_route))
    } else {
        (None, None)
    };
    Ok(PreparedUdpAttempt {
        lanes,
        quic,
        down_datagrams,
        pending_udp_route,
    })
}

#[allow(
    clippy::too_many_arguments,
    reason = "setup keeps the wire header fields and lane shape explicit"
)]
async fn setup_udp_lanes(
    lanes: &mut [PhysicalLane],
    flow_id: u32,
    route: ResolvedRoute,
    target: &crate::protocol::Target,
    hops: u8,
) -> std::result::Result<(), OpenFlowError> {
    let uplink = route.uplink;
    let downlink = route.downlink;
    let split_lanes = route.split();
    let open = FlowHeader {
        role: if split_lanes {
            FlowRole::Open
        } else {
            FlowRole::Duplex
        },
        flow_id,
        kind: FlowKind::Udp,
        uplink,
        downlink,
        hops,
    };
    let pending_auth = lanes[0].take_pending_auth();
    write_open_request(
        lanes[0].writer.as_mut().expect("uplink writer"),
        pending_auth,
        open,
        target,
    )
    .await
    .map_err(OpenFlowError::Transport)?;
    lanes[0].mark_auth_sent();
    if split_lanes {
        let pending_auth = lanes[1].take_pending_auth();
        write_header(
            lanes[1].writer.as_mut().expect("downlink writer"),
            pending_auth,
            FlowHeader {
                role: FlowRole::Attach,
                ..open
            },
        )
        .await
        .map_err(OpenFlowError::Transport)?;
        lanes[1].mark_auth_sent();
    }
    if uplink == Carrier::Quic {
        timeout(
            handshake_timeout(),
            lanes[0]
                .writer
                .as_mut()
                .expect("QUIC uplink control")
                .shutdown(),
        )
        .await
        .map_err(|_| {
            OpenFlowError::Transport(anyhow::anyhow!(
                "vector::udp_flow::setup_udp_lanes: uplink shutdown timeout"
            ))
        })?
        .map_err(|error| OpenFlowError::Transport(error.into()))?;
    }
    let down_index = usize::from(split_lanes);
    if downlink == Carrier::Quic && down_index != 0 {
        timeout(
            handshake_timeout(),
            lanes[down_index]
                .writer
                .as_mut()
                .expect("QUIC downlink control")
                .shutdown(),
        )
        .await
        .map_err(|_| {
            OpenFlowError::Transport(anyhow::anyhow!(
                "vector::udp_flow::setup_udp_lanes: downlink shutdown timeout"
            ))
        })?
        .map_err(|error| OpenFlowError::Transport(error.into()))?;
    }
    read_ready(lanes[down_index].reader.as_mut().expect("downlink reader"))
        .await
        .map_err(OpenFlowError::Setup)
}
