// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

//! Read-only Linux terminal dashboard for running Nowhere processes.

mod app;
mod client;
mod format;
mod input;
pub mod model;
mod render;

use anyhow::Result;

pub use app::run_with_receiver;
pub use client::UiCommand;

/// Discovers running Nowhere instances and opens the interactive dashboard.
pub async fn run() -> Result<()> {
    let client = client::start()?;
    app::run_with_receiver(client.events, Some(client.commands)).await
}
