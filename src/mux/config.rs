// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

use std::io;
use std::time::Duration;

use tokio::sync::Semaphore;

pub(super) const MIB: usize = 1024 * 1024;
pub(super) const BASE_STREAM_WINDOW_BYTES: usize = 4 * MIB;
pub(super) const BASE_CONNECTION_WINDOW_BYTES: usize = 8 * MIB;
pub(super) const MAX_STREAM_WINDOW_BYTES: usize = 16 * MIB;
pub(super) const MAX_CONNECTION_WINDOW_BYTES: usize = 32 * MIB;
pub(super) const CREDIT_UNIT_BYTES: usize = 1024;
pub(super) const WINDOW_UPDATE_DIVISOR: usize = 8;
const ACTIVE_STREAM_RESOURCE_LIMIT: usize = 4096;
pub(crate) const MUX_IDLE_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct MuxConfig {
    pub stream_window_bytes: usize,
    pub connection_window_bytes: usize,
    pub active_stream_limit: usize,
    pub outbound_frames: usize,
}

impl Default for MuxConfig {
    fn default() -> Self {
        Self::from_flow_control(crate::transport::transport_flow_control().unwrap_or(
            crate::transport::TransportFlowControl {
                stream_receive_window: MAX_STREAM_WINDOW_BYTES as u32,
                connection_receive_window: MAX_CONNECTION_WINDOW_BYTES as u32,
                send_window: MAX_CONNECTION_WINDOW_BYTES as u64,
            },
        ))
    }
}

impl MuxConfig {
    pub(crate) fn from_flow_control(profile: crate::transport::TransportFlowControl) -> Self {
        Self {
            stream_window_bytes: profile.stream_receive_window as usize,
            connection_window_bytes: profile.connection_receive_window as usize,
            active_stream_limit: ACTIVE_STREAM_RESOURCE_LIMIT,
            outbound_frames: 512,
        }
    }
    pub(super) fn validate(self) -> io::Result<Self> {
        if self.stream_window_bytes < BASE_STREAM_WINDOW_BYTES
            || self.stream_window_bytes > MAX_STREAM_WINDOW_BYTES
            || self.connection_window_bytes < BASE_CONNECTION_WINDOW_BYTES
            || self.connection_window_bytes > MAX_CONNECTION_WINDOW_BYTES
            || !self.stream_window_bytes.is_multiple_of(CREDIT_UNIT_BYTES)
            || !self
                .connection_window_bytes
                .is_multiple_of(CREDIT_UNIT_BYTES)
            || self.connection_window_bytes < self.stream_window_bytes
            || self.active_stream_limit == 0
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
