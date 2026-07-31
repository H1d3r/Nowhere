// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

//! Structured, process-local telemetry for the read-only TUI.
//!
//! This path is deliberately independent from [`crate::common::Logger`].
//! Portal and Vector continue to emit their existing stdout/stderr records;
//! the telemetry hub broadcasts structured state over local IPC.

mod hub;
mod ipc;
mod process;
mod wire;

pub(crate) use hub::{AccessSpan, TelemetryHub};
pub(crate) use ipc::{DiscoveredInstance, TelemetryClient, TelemetryServer, discover_instances};
pub(crate) use process::now_unix_ms;
pub(crate) use wire::{
    AccessFinished, AccessOutcome, AccessStart, AccessStarted, ClientMessage, Hello, InstanceRole,
    MAX_FRAME_SIZE, PROTOCOL_VERSION, RuntimeEvent, RuntimeKind, RuntimeLevel, ServerMessage,
    Subscription, TelemetrySnapshot, TrafficProtocol,
};
#[cfg(test)]
pub(crate) use wire::{InstanceDescriptor, LifecycleSnapshot};
