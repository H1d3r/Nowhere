// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

//! QUIC UDP route setup ownership and flow-ID reuse tests.

use super::*;

fn insert_pending_route(
    routes: &Arc<StdMutex<HashMap<FlowId, UdpRoute>>>,
    flow_id: FlowId,
) -> PendingUdpRoute {
    let (sender, _receiver) = mpsc::channel(1);
    let generation = Arc::new(());
    routes.lock().unwrap().insert(
        flow_id,
        UdpRoute {
            sender,
            ready: false,
            generation: generation.clone(),
        },
    );
    PendingUdpRoute {
        routes: Arc::downgrade(routes),
        flow_id,
        generation,
        armed: true,
    }
}

#[test]
fn cancelled_udp_setup_removes_pending_route() {
    let routes = Arc::new(StdMutex::new(HashMap::new()));
    let pending = insert_pending_route(&routes, 7);

    drop(pending);

    assert!(!routes.lock().unwrap().contains_key(&7));
}

#[test]
fn stale_udp_setup_does_not_remove_reused_flow_id() {
    let routes = Arc::new(StdMutex::new(HashMap::new()));
    let stale = insert_pending_route(&routes, 7);
    let current = insert_pending_route(&routes, 7);

    drop(stale);

    assert!(routes.lock().unwrap().contains_key(&7));
    drop(current);
    assert!(!routes.lock().unwrap().contains_key(&7));
}

#[test]
fn committed_udp_route_outlives_setup_guard() {
    let routes = Arc::new(StdMutex::new(HashMap::new()));
    let pending = insert_pending_route(&routes, 7);

    pending.commit();

    assert!(routes.lock().unwrap().contains_key(&7));
}
