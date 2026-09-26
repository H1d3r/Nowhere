// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

//! Shared imports and module wiring for Mux runtime tests.

use std::io;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use super::config::MIB;
use super::wire::{CLOSE_FIN, CLOSE_RESET, FrameHeader, encode_header};
use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

impl MuxHandle {
    pub(crate) async fn open_stream(&self, flow_id: FlowId) -> io::Result<MuxStream> {
        let stream = self.prepare_stream(flow_id)?;
        self.open_prepared(stream).await
    }

    pub(crate) fn contains_flow(&self, flow_id: FlowId) -> bool {
        self.shared
            .flows
            .lock()
            .expect("mux flow lock")
            .contains_key(&flow_id)
    }

    pub(crate) fn borrowed_write_copies(&self) -> usize {
        self.shared.borrowed_write_copies.load(Ordering::Relaxed)
    }

    pub(crate) fn reset_borrowed_write_copies(&self) {
        self.shared
            .borrowed_write_copies
            .store(0, Ordering::Relaxed);
    }

    pub(crate) fn same_carrier(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.shared, &other.shared)
    }
}

#[path = "runtime/admission.rs"]
mod admission;
#[path = "runtime/streams.rs"]
mod streams;
#[path = "runtime/wire_and_lifecycle.rs"]
mod wire_and_lifecycle;
