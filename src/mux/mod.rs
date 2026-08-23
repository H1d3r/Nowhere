// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use tokio::sync::{Notify, Semaphore, mpsc, oneshot, watch};

use self::driver::closed;
use self::wire::{FlowId, FrameHeader};

mod driver;
mod handle;
mod stream;
mod wire;

const FRAME_BYTES: usize = 32 * 1024;
const WINDOW_UPDATE_BYTES: usize = 4 * 1024;
// Frame count is separate from the byte window: UoT carries many small
// packets, so the frame queue absorbs scheduling bursts while the byte window
// remains the hard payload bound.
const FLOW_CHANNEL_FRAMES: usize = 512;
const MIN_FAIR_CREDIT_BYTES: usize = 256 * 1024;
pub(crate) const MUX_IDLE_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct MuxConfig {
    pub stream_window_bytes: usize,
    pub connection_window_bytes: usize,
    pub max_streams: usize,
    pub outbound_frames: usize,
}

impl Default for MuxConfig {
    fn default() -> Self {
        Self {
            stream_window_bytes: 512 * 1024,
            connection_window_bytes: 512 * 1024,
            max_streams: 256,
            outbound_frames: 512,
        }
    }
}

impl MuxConfig {
    fn validate(self) -> io::Result<Self> {
        if self.stream_window_bytes < FRAME_BYTES
            || self.connection_window_bytes < self.stream_window_bytes
            || self.max_streams == 0
            || self.outbound_frames == 0
            || self.connection_window_bytes > Semaphore::MAX_PERMITS
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid mux limits",
            ));
        }
        Ok(self)
    }
}

pub(crate) struct MuxStream {
    reader: FlowReader,
    writer: FlowWriter,
}

pub(crate) struct FlowReader {
    shared: Arc<Shared>,
    flow_id: FlowId,
    receiver: mpsc::Receiver<Inbound>,
    current: Option<(Bytes, usize, usize)>,
    eof: bool,
}

pub(crate) struct FlowWriter {
    shared: Arc<Shared>,
    flow_id: FlowId,
    pending: Option<WriteFuture>,
    pending_action: Option<ActionFuture>,
    terminal_permit: Option<mpsc::OwnedPermit<FlowId>>,
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

struct Shared {
    config: MuxConfig,
    flows: Mutex<HashMap<FlowId, FlowState>>,
    connection_send_credit: Arc<Semaphore>,
    connection_receive_credit: Mutex<usize>,
    pending_connection_credit: AtomicUsize,
    ready_flows: Mutex<VecDeque<FlowId>>,
    data_tx: mpsc::Sender<Outbound>,
    terminal_tx: mpsc::Sender<FlowId>,
    control_notify: Notify,
    incoming_tx: mpsc::Sender<MuxStream>,
    active_streams_tx: watch::Sender<usize>,
    closed: AtomicBool,
    closed_notify: Notify,
}

struct FlowState {
    inbound: mpsc::Sender<Inbound>,
    send_credit: Arc<Semaphore>,
    fair_send_credit: Arc<Semaphore>,
    fair_limit: usize,
    fair_debt: usize,
    receive_credit: usize,
    pending_receive_credit: usize,
    window_queued: bool,
    local_parts: u8,
}

enum Inbound {
    Data { payload: Bytes, charge: usize },
    Fin,
    Reset,
}

struct Outbound {
    header: FrameHeader,
    payload: Bytes,
    flushed: Option<oneshot::Sender<io::Result<()>>>,
}

impl Shared {
    async fn reserve_terminal(&self) -> io::Result<mpsc::OwnedPermit<FlowId>> {
        tokio::select! {
            _ = self.closed_notify.notified() => Err(closed()),
            permit = self.terminal_tx.clone().reserve_owned() => permit.map_err(|_| closed()),
        }
    }

    fn insert_flow(
        self: &Arc<Self>,
        flow_id: FlowId,
        terminal_permit: mpsc::OwnedPermit<FlowId>,
    ) -> io::Result<MuxStream> {
        if flow_id == 0 || self.closed.load(Ordering::Acquire) {
            return Err(closed());
        }
        let (sender, receiver) = mpsc::channel(FLOW_CHANNEL_FRAMES);
        let mut flows = self.flows.lock().expect("mux flow lock");
        if flows.len() >= self.config.max_streams {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "mux stream limit reached",
            ));
        }
        if flows.contains_key(&flow_id) {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "mux flow already exists",
            ));
        }
        let send_credit = Arc::new(Semaphore::new(self.config.stream_window_bytes));
        let fair_send_credit = Arc::new(Semaphore::new(self.config.stream_window_bytes));
        flows.insert(
            flow_id,
            FlowState {
                inbound: sender,
                send_credit,
                fair_send_credit,
                fair_limit: self.config.stream_window_bytes,
                fair_debt: 0,
                receive_credit: self.config.stream_window_bytes,
                pending_receive_credit: 0,
                window_queued: false,
                local_parts: 2,
            },
        );
        Self::rebalance_fair_credits(&mut flows, self.config);
        let active_streams = flows.len();
        drop(flows);
        self.active_streams_tx.send_replace(active_streams);
        Ok(MuxStream {
            reader: FlowReader {
                shared: self.clone(),
                flow_id,
                receiver,
                current: None,
                eof: false,
            },
            writer: FlowWriter {
                shared: self.clone(),
                flow_id,
                pending: None,
                pending_action: None,
                terminal_permit: Some(terminal_permit),
                closed: false,
            },
        })
    }

    fn close(&self) {
        if self.closed.swap(true, Ordering::AcqRel) {
            return;
        }
        self.flows.lock().expect("mux flow lock").clear();
        self.active_streams_tx.send_replace(0);
        self.closed_notify.notify_waiters();
    }

    fn send_credits(&self, flow_id: FlowId) -> io::Result<(Arc<Semaphore>, Arc<Semaphore>)> {
        self.flows
            .lock()
            .expect("mux flow lock")
            .get(&flow_id)
            .map(|flow| (flow.send_credit.clone(), flow.fair_send_credit.clone()))
            .ok_or_else(closed)
    }

    fn rebalance_fair_credits(flows: &mut HashMap<FlowId, FlowState>, config: MuxConfig) {
        if flows.is_empty() {
            return;
        }
        let fair_limit = (config.connection_window_bytes / flows.len())
            .max(MIN_FAIR_CREDIT_BYTES)
            .min(config.stream_window_bytes);
        for flow in flows.values_mut() {
            if fair_limit < flow.fair_limit {
                let reduction = flow.fair_limit - fair_limit;
                let removed = flow.fair_send_credit.forget_permits(reduction);
                flow.fair_debt = flow.fair_debt.saturating_add(reduction - removed);
            } else if fair_limit > flow.fair_limit {
                let increase = fair_limit - flow.fair_limit;
                let debt_repaid = increase.min(flow.fair_debt);
                flow.fair_debt -= debt_repaid;
                flow.fair_send_credit.add_permits(increase - debt_repaid);
            }
            flow.fair_limit = fair_limit;
        }
    }

    fn return_fair_credit(flow: &mut FlowState, credit: usize) {
        let debt_repaid = credit.min(flow.fair_debt);
        flow.fair_debt -= debt_repaid;
        let returned = credit - debt_repaid;
        let room = flow
            .fair_limit
            .saturating_sub(flow.fair_send_credit.available_permits());
        flow.fair_send_credit.add_permits(returned.min(room));
    }

    fn remove_flow(&self, flow_id: FlowId) -> Option<FlowState> {
        let mut flows = self.flows.lock().expect("mux flow lock");
        let removed = flows.remove(&flow_id);
        if removed.is_some() {
            Self::rebalance_fair_credits(&mut flows, self.config);
        }
        let active_streams = flows.len();
        drop(flows);
        self.active_streams_tx.send_replace(active_streams);
        removed
    }

    fn admit_receive(&self, flow_id: FlowId, charge: usize) -> io::Result<mpsc::Sender<Inbound>> {
        let mut connection = self
            .connection_receive_credit
            .lock()
            .expect("mux credit lock");
        let mut flows = self.flows.lock().expect("mux flow lock");
        let flow = flows.get_mut(&flow_id).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "frame for unknown mux flow")
        })?;
        if flow.receive_credit < charge || *connection < charge {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "peer exceeded mux window",
            ));
        }
        flow.receive_credit -= charge;
        *connection -= charge;
        Ok(flow.inbound.clone())
    }

    fn release_receive(&self, flow_id: FlowId, charge: usize) {
        if self.closed.load(Ordering::Acquire) {
            return;
        }
        let (flow_ready, flow_notify) = {
            let mut connection = self
                .connection_receive_credit
                .lock()
                .expect("mux credit lock");
            *connection = connection
                .saturating_add(charge)
                .min(self.config.connection_window_bytes);
            if let Some(flow) = self.flows.lock().expect("mux flow lock").get_mut(&flow_id) {
                flow.receive_credit = flow
                    .receive_credit
                    .saturating_add(charge)
                    .min(self.config.stream_window_bytes);
                flow.pending_receive_credit = flow.pending_receive_credit.saturating_add(charge);
                let ready = if flow.window_queued {
                    false
                } else {
                    flow.window_queued = true;
                    true
                };
                (ready, flow.pending_receive_credit >= WINDOW_UPDATE_BYTES)
            } else {
                return;
            }
        };
        if flow_ready {
            self.ready_flows
                .lock()
                .expect("mux ready-flow lock")
                .push_back(flow_id);
        }
        let previous = self
            .pending_connection_credit
            .fetch_add(charge, Ordering::AcqRel);
        if flow_notify || previous.saturating_add(charge) >= WINDOW_UPDATE_BYTES {
            self.control_notify.notify_one();
        }
    }

    fn release_part(&self, flow_id: FlowId) {
        let mut flows = self.flows.lock().expect("mux flow lock");
        let Some(flow) = flows.get_mut(&flow_id) else {
            return;
        };
        let flush_credit = flow.pending_receive_credit != 0;
        flow.local_parts = flow.local_parts.saturating_sub(1);
        if flow.local_parts == 0 {
            flows.remove(&flow_id);
            Self::rebalance_fair_credits(&mut flows, self.config);
        }
        let active_streams = flows.len();
        drop(flows);
        self.active_streams_tx.send_replace(active_streams);
        if flush_credit {
            self.control_notify.notify_one();
        }
    }
}

#[cfg(test)]
#[path = "../tests/mux/runtime.rs"]
mod tests;
