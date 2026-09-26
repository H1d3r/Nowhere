// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

//! Library entry point for the Nowhere Portal and Vector runtimes.

mod common;
mod mux;
mod portal;
mod protocol;
mod telemetry;
mod transport;
mod tui;
mod vector;

pub use common::{LogLevel, Logger, query_first, validate_endpoint_url_input};
pub use portal::Portal;
pub use tui::run_tui;
pub use vector::Vector;
