// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

//! Public Mux handle lifecycle and logical-stream admission.

use std::collections::{HashMap, VecDeque};
use std::io;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{Notify, Semaphore, mpsc, watch};

use super::driver::{closed, frame_stream, run_reader, run_terminals, run_writer};
use super::wire::FLAG_SYN;
use super::{Incoming, MuxConfig, MuxHandle, MuxStream, Outbound, Shared};

impl MuxHandle {
    pub(crate) fn start<T>(io: T, config: MuxConfig) -> io::Result<(Self, Incoming)>
    where
        T: AsyncRead + AsyncWrite + Send + Unpin + 'static,
    {
        let config = config.validate()?;
        let (data_tx, data_rx) = mpsc::channel(config.outbound_frames);
        let (terminal_tx, terminal_rx) = mpsc::channel(config.max_streams);
        let (incoming_tx, incoming_rx) = mpsc::channel(config.max_streams);
        let (active_streams_tx, _) = watch::channel(0);
        let shared = Arc::new(Shared {
            config,
            flows: Mutex::new(HashMap::new()),
            connection_send_credit: Arc::new(Semaphore::new(config.connection_window_bytes)),
            connection_receive_credit: Mutex::new(config.connection_window_bytes),
            pending_connection_credit: AtomicUsize::new(0),
            ready_flows: Mutex::new(VecDeque::with_capacity(config.max_streams)),
            data_tx,
            terminal_tx,
            control_notify: Notify::new(),
            incoming_tx,
            active_streams_tx,
            closed: AtomicBool::new(false),
            closed_notify: Notify::new(),
        });
        let (reader, writer) = tokio::io::split(io);
        tokio::spawn(run_reader(reader, shared.clone()));
        tokio::spawn(run_writer(writer, shared.clone(), data_rx));
        tokio::spawn(run_terminals(shared.clone(), terminal_rx));
        Ok((
            Self { shared },
            Incoming {
                receiver: incoming_rx,
            },
        ))
    }

    pub(crate) async fn open_stream(&self, flow_id: super::FlowId) -> io::Result<MuxStream> {
        let terminal_permit = self.shared.reserve_terminal().await?;
        let stream = self.shared.insert_flow(flow_id, terminal_permit)?;
        self.shared
            .data_tx
            .send(Outbound {
                header: frame_stream(flow_id, FLAG_SYN, 0)?,
                payload: Bytes::new(),
                flushed: None,
            })
            .await
            .map_err(|_| closed())?;
        Ok(stream)
    }

    pub(crate) fn is_closed(&self) -> bool {
        self.shared.closed.load(Ordering::Acquire)
    }

    pub(crate) fn active_streams(&self) -> usize {
        self.shared.flows.lock().expect("mux flow lock").len()
    }

    pub(crate) fn close(&self) {
        self.shared.close();
    }

    pub(crate) fn same_carrier(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.shared, &other.shared)
    }

    pub(crate) async fn idle_for(&self, duration: Duration) -> bool {
        let mut active = self.shared.active_streams_tx.subscribe();
        loop {
            if self.is_closed() {
                return false;
            }
            if *active.borrow_and_update() != 0 {
                if active.changed().await.is_err() {
                    return false;
                }
                continue;
            }
            tokio::select! {
                _ = tokio::time::sleep(duration) => {
                    if *active.borrow() == 0 && !self.is_closed() {
                        return true;
                    }
                }
                changed = active.changed() => {
                    if changed.is_err() {
                        return false;
                    }
                }
            }
        }
    }

    pub(crate) async fn closed(&self) {
        if self.is_closed() {
            return;
        }
        self.shared.closed_notify.notified().await;
    }
}

impl Incoming {
    pub(crate) async fn accept(&mut self) -> io::Result<Option<MuxStream>> {
        Ok(self.receiver.recv().await)
    }
}
