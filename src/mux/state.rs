// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

//! Shared Mux flow state, queues, and credit accounting.

use std::collections::{HashMap, VecDeque};
use std::io;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use tokio::sync::{Notify, Semaphore, mpsc, oneshot, watch};

use super::config::{BASE_STREAM_WINDOW_BYTES, WINDOW_UPDATE_DIVISOR};
use super::driver::closed;
use super::wire::{FlowId, FrameHeader};
use super::{FlowReader, FlowWriter, MuxChunk, MuxConfig, MuxStream, credit_units};

pub(super) struct Shared {
    pub(super) config: MuxConfig,
    pub(super) flows: Mutex<HashMap<FlowId, FlowState>>,
    pub(super) connection_send_credit: Arc<Semaphore>,
    pub(super) connection_send_peak: AtomicUsize,
    pub(super) connection_receive_credit: Mutex<usize>,
    pub(super) pending_connection_credit: AtomicUsize,
    pub(super) ready_flows: Mutex<VecDeque<FlowId>>,
    pub(super) data_tx: mpsc::Sender<Outbound>,
    pub(super) terminal_tx: mpsc::Sender<Terminal>,
    pub(super) control_notify: Notify,
    pub(super) incoming_tx: mpsc::Sender<MuxStream>,
    pub(super) active_streams_tx: watch::Sender<usize>,
    pub(super) closed: AtomicBool,
    pub(super) closed_notify: tokio_util::sync::CancellationToken,
    #[cfg(test)]
    pub(super) borrowed_write_copies: AtomicUsize,
}

pub(super) struct FlowState {
    pub(super) generation: Arc<()>,
    pub(super) inbound: mpsc::UnboundedSender<Inbound>,
    pub(super) send_credit: Arc<Semaphore>,
    pub(super) send_slot: Arc<Semaphore>,
    pub(super) receive_credit: usize,
    pub(super) pending_receive_credit: usize,
    pub(super) window_queued: bool,
    pub(super) local_parts: u8,
    pub(super) local_fin_sent: bool,
    pub(super) remote_fin: bool,
}

pub(super) enum Inbound {
    Data { payload: Bytes, charge: usize },
    Fin,
    Reset,
}

pub(super) enum ReceiveTarget {
    Deliver {
        inbound: mpsc::UnboundedSender<Inbound>,
        generation: Arc<()>,
    },
    Discard,
}

pub(super) struct Terminal {
    pub(super) flow_id: FlowId,
    pub(super) generation: Arc<()>,
}

pub(super) enum Outbound {
    Data {
        header: FrameHeader,
        payload: MuxChunk,
        generation: Arc<()>,
        _slot: tokio::sync::OwnedSemaphorePermit,
    },
    Control {
        header: FrameHeader,
        generation: Arc<()>,
        finishes_flow: bool,
    },
    Flush(oneshot::Sender<io::Result<()>>),
}

impl Shared {
    pub(super) fn insert_flow(
        self: &Arc<Self>,
        flow_id: FlowId,
        advertise_window: bool,
    ) -> io::Result<MuxStream> {
        if flow_id == 0 || flow_id > crate::protocol::MAX_FLOW_ID {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "flow ID is outside the 30-bit range",
            ));
        }
        if self.closed.load(Ordering::Acquire) {
            return Err(closed());
        }
        let (sender, receiver) = mpsc::unbounded_channel();
        let mut flows = self.flows.lock().expect("mux flow lock");
        if self.closed.load(Ordering::Acquire) {
            return Err(closed());
        }
        if flows.contains_key(&flow_id) {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "mux flow already exists",
            ));
        }
        if flows.len() >= self.config.active_stream_limit {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "mux active-stream resource limit reached",
            ));
        }
        let send_credit = Arc::new(Semaphore::new(credit_units(BASE_STREAM_WINDOW_BYTES)));
        let generation = Arc::new(());
        let initial_credit = if advertise_window {
            credit_units(
                self.config
                    .stream_window_bytes
                    .saturating_sub(BASE_STREAM_WINDOW_BYTES),
            )
        } else {
            0
        };
        flows.insert(
            flow_id,
            FlowState {
                generation: generation.clone(),
                inbound: sender,
                send_credit,
                send_slot: Arc::new(Semaphore::new(1)),
                receive_credit: credit_units(self.config.stream_window_bytes),
                pending_receive_credit: initial_credit,
                window_queued: advertise_window && initial_credit != 0,
                local_parts: 2,
                local_fin_sent: false,
                remote_fin: false,
            },
        );
        let active_streams = active_flow_count(&flows);
        self.active_streams_tx.send_replace(active_streams);
        drop(flows);
        if advertise_window && initial_credit != 0 {
            self.ready_flows
                .lock()
                .expect("mux ready-flow lock")
                .push_back(flow_id);
            self.control_notify.notify_one();
        }
        Ok(MuxStream {
            reader: FlowReader {
                shared: self.clone(),
                flow_id,
                generation: generation.clone(),
                receiver,
                current: None,
                eof: false,
            },
            writer: FlowWriter {
                shared: self.clone(),
                flow_id,
                generation,
                pending: None,
                pending_action: None,
                closed: false,
            },
        })
    }

    pub(super) fn close(&self) {
        if self.closed.swap(true, Ordering::AcqRel) {
            return;
        }
        let mut flows = self.flows.lock().expect("mux flow lock");
        for flow in flows.values() {
            flow.send_credit.close();
            flow.send_slot.close();
        }
        flows.clear();
        self.active_streams_tx.send_replace(0);
        drop(flows);
        self.connection_send_credit.close();
        self.closed_notify.cancel();
    }

    pub(super) fn send_credit(&self, flow_id: FlowId) -> io::Result<Arc<Semaphore>> {
        self.flows
            .lock()
            .expect("mux flow lock")
            .get(&flow_id)
            .map(|flow| flow.send_credit.clone())
            .ok_or_else(closed)
    }

    pub(super) fn is_current_flow(&self, flow_id: FlowId, expected: &Arc<()>) -> bool {
        self.flows
            .lock()
            .expect("mux flow lock")
            .get(&flow_id)
            .is_some_and(|flow| Arc::ptr_eq(&flow.generation, expected))
    }

    pub(super) fn remove_flow(&self, flow_id: FlowId) -> Option<FlowState> {
        let mut flows = self.flows.lock().expect("mux flow lock");
        let removed = flows.remove(&flow_id);
        if let Some(flow) = &removed {
            flow.send_credit.close();
            flow.send_slot.close();
        }
        let active_streams = active_flow_count(&flows);
        self.active_streams_tx.send_replace(active_streams);
        drop(flows);
        removed
    }

    pub(super) fn admit_receive(
        &self,
        flow_id: FlowId,
        charge: usize,
    ) -> io::Result<ReceiveTarget> {
        let mut connection = self
            .connection_receive_credit
            .lock()
            .expect("mux credit lock");
        let mut flows = self.flows.lock().expect("mux flow lock");
        let flow = flows.get_mut(&flow_id).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "frame for unknown mux flow")
        })?;
        if flow.remote_fin {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "DATA received after mux FIN",
            ));
        }
        if flow.receive_credit < charge || *connection < charge {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "peer exceeded mux window",
            ));
        }
        flow.receive_credit -= charge;
        *connection -= charge;
        if flow.local_parts == 0 {
            Ok(ReceiveTarget::Discard)
        } else {
            Ok(ReceiveTarget::Deliver {
                inbound: flow.inbound.clone(),
                generation: flow.generation.clone(),
            })
        }
    }

    pub(super) fn release_connection_receive(&self, charge: usize) {
        if self.closed.load(Ordering::Acquire) {
            return;
        }
        let mut connection = self
            .connection_receive_credit
            .lock()
            .expect("mux credit lock");
        *connection = connection
            .saturating_add(charge)
            .min(credit_units(self.config.connection_window_bytes));
        drop(connection);
        let previous = self
            .pending_connection_credit
            .fetch_add(charge, Ordering::AcqRel);
        let threshold = credit_units(self.config.connection_window_bytes / WINDOW_UPDATE_DIVISOR)
            .min(u16::MAX as usize);
        if previous.saturating_add(charge) >= threshold {
            self.control_notify.notify_one();
        }
    }

    pub(super) fn release_receive(&self, flow_id: FlowId, generation: &Arc<()>, charge: usize) {
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
                .min(credit_units(self.config.connection_window_bytes));
            if let Some(flow) = self
                .flows
                .lock()
                .expect("mux flow lock")
                .get_mut(&flow_id)
                .filter(|flow| flow.local_parts != 0 && Arc::ptr_eq(&flow.generation, generation))
            {
                flow.receive_credit = flow
                    .receive_credit
                    .saturating_add(charge)
                    .min(credit_units(self.config.stream_window_bytes));
                flow.pending_receive_credit = flow.pending_receive_credit.saturating_add(charge);
                let ready = if flow.window_queued {
                    false
                } else {
                    flow.window_queued = true;
                    true
                };
                let threshold =
                    credit_units(self.config.stream_window_bytes / WINDOW_UPDATE_DIVISOR)
                        .min(u16::MAX as usize);
                (ready, flow.pending_receive_credit >= threshold)
            } else {
                (false, false)
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
        let threshold = credit_units(self.config.connection_window_bytes / WINDOW_UPDATE_DIVISOR)
            .min(u16::MAX as usize);
        if flow_notify || previous.saturating_add(charge) >= threshold {
            self.control_notify.notify_one();
        }
    }

    pub(super) fn release_part(&self, flow_id: FlowId, generation: &Arc<()>) {
        let mut flows = self.flows.lock().expect("mux flow lock");
        let Some(flow) = flows.get_mut(&flow_id) else {
            return;
        };
        if !Arc::ptr_eq(&flow.generation, generation) {
            return;
        }
        flow.local_parts = flow.local_parts.saturating_sub(1);
        let flush_credit = flow.pending_receive_credit != 0;
        if flow.local_parts == 0 {
            flow.pending_receive_credit = 0;
            flow.window_queued = false;
        }
        if flow.local_parts == 0 && flow.remote_fin && flow.local_fin_sent {
            flows.remove(&flow_id);
        }
        let active_streams = active_flow_count(&flows);
        self.active_streams_tx.send_replace(active_streams);
        drop(flows);
        if flush_credit {
            self.control_notify.notify_one();
        }
    }

    pub(super) fn finish_local_fin(&self, flow_id: FlowId, generation: &Arc<()>) {
        let mut flows = self.flows.lock().expect("mux flow lock");
        if let Some(flow) = flows
            .get_mut(&flow_id)
            .filter(|flow| Arc::ptr_eq(&flow.generation, generation))
        {
            flow.local_fin_sent = true;
            if flow.local_parts == 0 && flow.remote_fin {
                flows.remove(&flow_id);
            }
        }
    }
}

pub(super) fn active_flow_count(flows: &HashMap<FlowId, FlowState>) -> usize {
    flows.values().filter(|flow| flow.local_parts != 0).count()
}
