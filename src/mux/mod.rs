// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

//! Multiplexed logical streams over a shared reliable carrier.

use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::Arc;

use bytes::Bytes;
use tokio::sync::mpsc;

#[cfg(test)]
use self::config::MIB;
use self::config::{
    BASE_CONNECTION_WINDOW_BYTES, BASE_STREAM_WINDOW_BYTES, CREDIT_UNIT_BYTES,
    MAX_CONNECTION_WINDOW_BYTES, MAX_STREAM_WINDOW_BYTES,
};
pub(crate) use self::config::{MUX_IDLE_TIMEOUT, MuxConfig};
use self::state::{Inbound, Outbound, ReceiveTarget, Shared, Terminal, active_flow_count};
use self::wire::FlowId;
#[cfg(test)]
use std::sync::atomic::Ordering;
#[cfg(test)]
use std::time::Duration;

mod config;
mod driver;
mod handle;
mod state;
mod stream;
mod wire;

pub(crate) const FRAME_BYTES: usize = 32 * 1024;
pub(crate) struct MuxStream {
    reader: FlowReader,
    writer: FlowWriter,
}

pub(crate) struct MuxChunk {
    payload: Bytes,
    _credit: Option<ReceiveCredit>,
}

struct ReceiveCredit {
    shared: Arc<Shared>,
    flow_id: FlowId,
    generation: Arc<()>,
    charge: usize,
}

pub(crate) struct FlowReader {
    shared: Arc<Shared>,
    flow_id: FlowId,
    generation: Arc<()>,
    receiver: mpsc::UnboundedReceiver<Inbound>,
    current: Option<(Bytes, usize, usize)>,
    eof: bool,
}

pub(crate) struct FlowWriter {
    shared: Arc<Shared>,
    flow_id: FlowId,
    generation: Arc<()>,
    pending: Option<WriteFuture>,
    pending_action: Option<ActionFuture>,
    closed: bool,
}

#[derive(Clone)]
pub(crate) struct MuxHandle {
    shared: Arc<Shared>,
}

pub(crate) struct Incoming {
    receiver: mpsc::Receiver<MuxStream>,
}

type WriteFuture = Pin<Box<dyn Future<Output = io::Result<usize>> + Send>>;
type ActionFuture = Pin<Box<dyn Future<Output = io::Result<()>> + Send>>;

impl MuxChunk {
    pub(crate) fn from_bytes(payload: Bytes) -> Self {
        Self {
            payload,
            _credit: None,
        }
    }

    fn received(
        payload: Bytes,
        shared: Arc<Shared>,
        flow_id: FlowId,
        generation: Arc<()>,
        charge: usize,
    ) -> Self {
        Self {
            payload,
            _credit: Some(ReceiveCredit {
                shared,
                flow_id,
                generation,
                charge,
            }),
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.payload.len()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.payload.is_empty()
    }
}

impl AsRef<[u8]> for MuxChunk {
    fn as_ref(&self) -> &[u8] {
        &self.payload
    }
}

impl Drop for ReceiveCredit {
    fn drop(&mut self) {
        self.shared
            .release_receive(self.flow_id, &self.generation, self.charge);
    }
}

fn credit_units(bytes: usize) -> usize {
    bytes.div_ceil(CREDIT_UNIT_BYTES)
}

#[cfg(test)]
#[path = "../tests/mux/runtime.rs"]
mod tests;
