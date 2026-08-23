// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

//! Configurable ALPN and the TLS Mux wire marker.

use anyhow::{Result, bail};

pub(crate) const DEFAULT_ALPN: &str = "now/1";
pub(crate) const MUX_MARKER: u8 = 0xff;

pub(crate) fn parse_alpn(value: Option<&str>) -> Result<String> {
    let value = value.unwrap_or(DEFAULT_ALPN);
    if value.is_empty() || value.len() > u8::MAX as usize {
        bail!("alpn length must be 1..255 bytes");
    }
    Ok(value.to_owned())
}

#[cfg(test)]
#[path = "../tests/common/alpn.rs"]
mod tests;
