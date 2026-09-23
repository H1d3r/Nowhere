// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

//! Session-global logical-flow registry and bounded half pairing.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use tokio::io::AsyncWriteExt;
use tokio::sync::{Mutex, Semaphore};

use crate::protocol::{
    Carrier, FlowErrorCode, FlowHeader, FlowKind, FlowResult, FlowRole, Target, write_flow_result,
};

mod admission;
mod lifecycle;
mod link;
mod state;
mod tcp;
mod udp;

pub(in crate::portal) use self::link::LinkGuard;
pub(super) use self::link::{guarded_reader, guarded_writer};
pub(super) use self::state::{
    BoxReader, BoxWriter, FlowLease, LinkHalf, LinkPath, PairedTcp, PairedUdp, QuicUdpReceiver,
    SessionKey, UdpDown, UdpHalf, UdpUp,
};
use self::state::{FlowClaim, FlowKey, LinkCounts, Metadata, PendingTcp, PendingUdp};
use self::tcp::reject_tcp_writer;
use self::udp::reject_udp_downlink_ref;

const FLOW_RESULT_WRITE_TIMEOUT: Duration = Duration::from_secs(1);
const SESSION_FLOW_RESOURCE_LIMIT: usize = 4096;
const PORTAL_FLOW_RESOURCE_LIMIT: usize = 65_536;

#[derive(Clone, Copy)]
struct TerminalRejection {
    code: FlowErrorCode,
    expires_at: Instant,
}

#[derive(Debug)]
pub(super) struct PairingError {
    code: FlowErrorCode,
    message: &'static str,
}

impl PairingError {
    fn new(code: FlowErrorCode, message: &'static str) -> Self {
        Self { code, message }
    }

    pub(super) fn code(&self) -> FlowErrorCode {
        self.code
    }
}

impl fmt::Display for PairingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message)
    }
}

impl std::error::Error for PairingError {}

pub(super) struct PairingRegistry {
    pub(super) tcp: Mutex<HashMap<FlowKey, PendingTcp>>,
    pub(super) udp: Mutex<HashMap<FlowKey, PendingUdp>>,
    pub(super) links: StdMutex<HashMap<SessionKey, LinkCounts>>,
    claims: StdMutex<HashMap<FlowKey, FlowClaim>>,
    claim_admission: Arc<Semaphore>,
    rejections: StdMutex<HashMap<FlowKey, TerminalRejection>>,
    accepting: AtomicBool,
    pub(super) next_quic_generation: AtomicU64,
    next_epoch: AtomicU64,
    pub(super) timeout: Duration,
}

const MAX_REJECTION_TOMBSTONES: usize = 1024;

impl PairingRegistry {
    pub(super) fn new(timeout: Duration) -> Self {
        Self {
            tcp: Mutex::new(HashMap::new()),
            udp: Mutex::new(HashMap::new()),
            links: StdMutex::new(HashMap::new()),
            claims: StdMutex::new(HashMap::new()),
            claim_admission: Arc::new(Semaphore::new(PORTAL_FLOW_RESOURCE_LIMIT)),
            rejections: StdMutex::new(HashMap::new()),
            accepting: AtomicBool::new(true),
            next_quic_generation: AtomicU64::new(1),
            next_epoch: AtomicU64::new(1),
            timeout,
        }
    }

    fn active_quic_generation(&self, session_id: SessionKey) -> Option<u64> {
        self.links
            .lock()
            .expect("link registry poisoned")
            .get(&session_id)
            .and_then(|counts| counts.udp.as_ref().map(|active| active.generation))
    }

    fn validate_current_link_locked(
        &self,
        session_id: SessionKey,
        link: &LinkHalf,
        links: &HashMap<SessionKey, LinkCounts>,
    ) -> Result<(), PairingError> {
        let current = links.get(&session_id);
        let valid = match link.quic_generation {
            Some(generation) => current
                .and_then(|counts| counts.udp.as_ref())
                .is_some_and(|active| active.generation == generation),
            None => current.is_some_and(|counts| counts.tcp > 0),
        };
        if valid {
            Ok(())
        } else {
            Err(PairingError::new(
                FlowErrorCode::SessionReplaced,
                "portal::pairing: carrier replaced before flow installation",
            ))
        }
    }

    fn validate_header_and_link(
        &self,
        session_id: SessionKey,
        header: FlowHeader,
        expected_kind: FlowKind,
        target: Option<&Target>,
        link: &LinkHalf,
    ) -> Result<(), PairingError> {
        if header.kind != expected_kind {
            return Err(PairingError::new(
                FlowErrorCode::InvalidRequest,
                "portal::pairing: flow kind mismatch",
            ));
        }
        match header.role {
            FlowRole::Open | FlowRole::Duplex if target.is_none() => {
                return Err(PairingError::new(
                    FlowErrorCode::InvalidRequest,
                    "portal::pairing: missing target",
                ));
            }
            FlowRole::Attach if target.is_some() => {
                return Err(PairingError::new(
                    FlowErrorCode::InvalidRequest,
                    "portal::pairing: attach target",
                ));
            }
            _ => {}
        }
        let carrier = match header.role {
            FlowRole::Open => header.uplink,
            FlowRole::Attach => header.downlink,
            FlowRole::Duplex => header.uplink,
        };
        let uses_quic = carrier == Carrier::Quic;
        if uses_quic != link.quic_generation.is_some()
            || link.quic_generation.is_some_and(|generation| {
                self.active_quic_generation(session_id) != Some(generation)
            })
        {
            return Err(PairingError::new(
                FlowErrorCode::SessionReplaced,
                "portal::pairing: stale or missing QUIC generation",
            ));
        }
        Ok(())
    }

    pub(super) async fn reject_flow_setup<S: Into<SessionKey>>(
        self: &Arc<Self>,
        session_id: S,
        flow_id: u32,
        code: FlowErrorCode,
    ) {
        let session_id = session_id.into();
        let key = FlowKey {
            session_id,
            flow_id,
        };
        let (mut tcp_downlink, udp_downlink) = {
            let mut tcp = self.tcp.lock().await;
            let mut udp = self.udp.lock().await;
            let mut claims = self.claims.lock().expect("flow claim registry poisoned");
            if claims.get(&key).is_some_and(|claim| claim.active) {
                return;
            }
            let tcp_downlink = tcp.remove(&key).and_then(|mut flow| flow.downlink.take());
            let udp_downlink = udp.remove(&key).and_then(|mut flow| flow.downlink.take());
            claims.remove(&key);
            if tcp_downlink.is_none() && udp_downlink.is_none() {
                let expires_at = Instant::now() + self.timeout;
                let mut rejections = self
                    .rejections
                    .lock()
                    .expect("flow rejection registry poisoned");
                let now = Instant::now();
                rejections.retain(|_, rejection| rejection.expires_at > now);
                if !rejections.contains_key(&key)
                    && rejections.len() >= MAX_REJECTION_TOMBSTONES
                    && let Some(oldest) = rejections
                        .iter()
                        .min_by_key(|(_, rejection)| rejection.expires_at)
                        .map(|(key, _)| *key)
                {
                    rejections.remove(&oldest);
                }
                rejections.insert(key, TerminalRejection { code, expires_at });
            }
            (tcp_downlink, udp_downlink)
        };
        reject_tcp_writer(&mut tcp_downlink, code).await;
        if let Some(mut downlink) = udp_downlink {
            reject_udp_downlink_ref(&mut downlink, code).await;
        }
    }
}

#[cfg(test)]
#[path = "../tests/portal/pairing.rs"]
mod tests;
