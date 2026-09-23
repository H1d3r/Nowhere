// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

//! Test-only inspection of unauthenticated admission counters.

use super::*;

impl UnauthenticatedAdmission {
    pub(in crate::portal) fn active(&self) -> usize {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .total
    }
}
