// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

use super::*;

impl PairingRegistry {
    pub(super) fn reserve_claim(
        &self,
        key: FlowKey,
        metadata: Metadata,
        target: Option<Target>,
        quic_generation: Option<u64>,
        quic_count: Option<Arc<AtomicUsize>>,
        session_admission: Option<Arc<Semaphore>>,
    ) -> Result<(u64, bool), PairingError> {
        let mut claims = self.claims.lock().expect("flow claim registry poisoned");
        // The claims lock is the drain/admission linearization point. Once
        // draining flips this flag while holding the same lock, neither an
        // OPEN nor a late ATTACH can create or complete another flow.
        if !self.accepting.load(Ordering::Acquire) {
            return Err(PairingError::new(
                FlowErrorCode::FlowLimit,
                "portal::pairing: portal is draining",
            ));
        }
        if let Some(claim) = claims.get_mut(&key) {
            if claim.active || claim.metadata != metadata {
                return Err(PairingError::new(
                    FlowErrorCode::MetadataConflict,
                    "portal::pairing: flow id metadata collision",
                ));
            }
            if let (Some(existing), Some(incoming)) = (&claim.target, &target)
                && existing != incoming
            {
                return Err(PairingError::new(
                    FlowErrorCode::MetadataConflict,
                    "portal::pairing: conflicting flow target",
                ));
            }
            if claim.target.is_none() {
                claim.target = target;
            }
            if let Some(generation) = quic_generation
                && !claim.quic_generations.contains(&generation)
            {
                claim.quic_generations.push(generation);
            }
            return Ok((claim.epoch, false));
        }
        let epoch = self.next_epoch.fetch_add(1, Ordering::Relaxed);
        let session_admission = session_admission.ok_or_else(|| {
            PairingError::new(
                FlowErrorCode::SessionReplaced,
                "portal::pairing: session disappeared before flow admission",
            )
        })?;
        let session_admission = session_admission.try_acquire_owned().map_err(|_| {
            PairingError::new(
                FlowErrorCode::FlowLimit,
                "portal::pairing: session flow resource limit reached",
            )
        })?;
        let portal_admission = self
            .claim_admission
            .clone()
            .try_acquire_owned()
            .map_err(|_| {
                PairingError::new(
                    FlowErrorCode::FlowLimit,
                    "portal::pairing: Portal flow resource limit reached",
                )
            })?;
        let quic_count = quic_count
            .filter(|_| metadata.uplink == Carrier::Quic || metadata.downlink == Carrier::Quic);
        if let Some(count) = &quic_count {
            count.fetch_add(1, Ordering::Relaxed);
        }
        claims.insert(
            key,
            FlowClaim {
                _portal_admission: portal_admission,
                _session_admission: session_admission,
                quic_count,
                epoch,
                metadata,
                target,
                active: false,
                cancel: tokio_util::sync::CancellationToken::new(),
                quic_generations: quic_generation.into_iter().collect(),
            },
        );
        Ok((epoch, true))
    }

    pub(super) fn quic_flow_counter(&self, session_id: SessionKey) -> Option<Arc<AtomicUsize>> {
        self.links
            .lock()
            .expect("link registry poisoned")
            .get(&session_id)
            .map(|counts| counts.quic_flows.clone())
    }

    pub(super) fn session_flow_admission(&self, session_id: SessionKey) -> Option<Arc<Semaphore>> {
        self.links
            .lock()
            .expect("link registry poisoned")
            .get(&session_id)
            .map(|counts| counts.flow_admission.clone())
    }

    pub(in crate::portal) fn quic_stream_credit(&self, session_id: SessionKey) -> quinn::VarInt {
        // Quinn preallocates stream state. Keep a sliding headroom for setup,
        // instead of advertising a huge fixed count or capping active flows.
        let live = self
            .quic_flow_counter(session_id)
            .map_or(0, |count| count.load(Ordering::Relaxed));
        // Quinn batches MAX_STREAMS updates at 1/8 of its window. Headroom
        // must grow too, otherwise a fixed reserve eventually stalls updates.
        quinn::VarInt::from_u32(
            live.saturating_add((live / 4).max(64))
                .min(SESSION_FLOW_RESOURCE_LIMIT) as u32,
        )
    }

    pub(super) fn refresh_claim(&self, key: FlowKey) -> Result<u64, PairingError> {
        let epoch = self.next_epoch.fetch_add(1, Ordering::Relaxed);
        let mut claims = self.claims.lock().expect("flow claim registry poisoned");
        let claim = claims.get_mut(&key).ok_or_else(|| {
            PairingError::new(
                FlowErrorCode::InternalError,
                "portal::pairing: missing pending flow claim",
            )
        })?;
        claim.epoch = epoch;
        Ok(epoch)
    }

    pub(super) fn abandon_claim(&self, key: FlowKey, epoch: u64) {
        let mut claims = self.claims.lock().expect("flow claim registry poisoned");
        if claims
            .get(&key)
            .is_some_and(|claim| !claim.active && claim.epoch == epoch)
        {
            claims.remove(&key);
        }
    }

    pub(super) fn activate_claim(
        self: &Arc<Self>,
        key: FlowKey,
        epoch: u64,
        quic_generations: Vec<u64>,
    ) -> Result<FlowLease, PairingError> {
        // `links -> claims` is the linearization barrier shared with QUIC
        // replacement.  A generation cannot become active after it has been
        // replaced, and replacement cannot miss a claim that just activated.
        let links = self.links.lock().expect("link registry poisoned");
        let active_generation = links
            .get(&key.session_id)
            .and_then(|counts| counts.udp.as_ref().map(|active| active.generation));
        if quic_generations
            .iter()
            .any(|generation| Some(*generation) != active_generation)
        {
            return Err(PairingError::new(
                FlowErrorCode::SessionReplaced,
                "portal::pairing: QUIC generation replaced before activation",
            ));
        }
        let cancel = {
            let mut claims = self.claims.lock().expect("flow claim registry poisoned");
            if !self.accepting.load(Ordering::Acquire) {
                return Err(PairingError::new(
                    FlowErrorCode::FlowLimit,
                    "portal::pairing: portal is draining",
                ));
            }
            let claim = claims.get_mut(&key).ok_or_else(|| {
                PairingError::new(
                    FlowErrorCode::InternalError,
                    "portal::pairing: missing flow claim",
                )
            })?;
            claim.epoch = epoch;
            claim.active = true;
            // A pending claim can survive a QUIC-carrier replacement while its
            // TLS/TCP half remains valid.  Once pairing completes, ownership
            // must describe only the carriers that formed this flow; otherwise
            // dropping the replaced carrier can cancel the new flow.
            claim.quic_generations = quic_generations;
            claim.cancel.clone()
        };
        drop(links);
        Ok(FlowLease {
            registry: Arc::downgrade(self),
            key,
            epoch,
            cancel,
        })
    }

    pub(super) fn terminal_rejection(&self, key: FlowKey, consume: bool) -> Option<FlowErrorCode> {
        let now = Instant::now();
        let mut rejections = self
            .rejections
            .lock()
            .expect("flow rejection registry poisoned");
        rejections.retain(|_, rejection| rejection.expires_at > now);
        if consume {
            rejections.remove(&key).map(|rejection| rejection.code)
        } else {
            rejections.get(&key).map(|rejection| rejection.code)
        }
    }
}
