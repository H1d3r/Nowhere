// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

use super::*;

#[test]
fn forwarding_initializes_decrements_and_exhausts_the_fixed_budget() {
    assert_eq!(forwarded_hops(0).unwrap(), MAX_PORTAL_HOPS);
    assert_eq!(forwarded_hops(7).unwrap(), 6);
    assert_eq!(forwarded_hops(2).unwrap(), 1);
    assert_eq!(
        forwarded_hops(1).unwrap_err().setup_result(),
        Some(SetupResult::FlowLimit)
    );
}

#[test]
fn every_upstream_rejection_keeps_its_exact_setup_code() {
    for result in [
        SetupResult::InvalidRequest,
        SetupResult::MetadataConflict,
        SetupResult::PairTimeout,
        SetupResult::FlowLimit,
        SetupResult::DialFailed,
        SetupResult::SessionReplaced,
        SetupResult::InternalError,
    ] {
        let error = OutboundError::flow(OpenFlowError::Setup(result));
        assert_eq!(error.setup_result(), Some(result));
    }
}
