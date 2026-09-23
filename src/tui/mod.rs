// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

//! Read-only terminal dashboard for running Nowhere processes.

mod app;
mod client;
mod client_adapter;
mod format;
mod input;
pub mod model;
mod render;

use anyhow::Result;

pub use app::run_with_receiver;
pub use client::UiCommand;

pub async fn run() -> Result<()> {
    let client = client::start()?;
    app::run_with_receiver(client.events, Some(client.commands)).await
}
