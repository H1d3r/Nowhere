// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

//! Library entry point for the Nowhere Portal and Vector runtimes.

#[cfg(not(target_os = "linux"))]
compile_error!("Nowhere supports Linux only");

pub mod common;
pub(crate) mod mux;
pub mod portal;
pub mod protocol;
pub(crate) mod telemetry;
pub mod transport;
pub mod tui;
pub mod vector;
