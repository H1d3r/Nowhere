// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

//! Configurable ALPN and TLS Mux mode shared by Portal and Vector.

use std::fmt;

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

/// Whether this endpoint may originate or accept TLS Mux carriers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MuxMode {
    Disabled,
    Enabled,
}

impl MuxMode {
    pub(crate) fn parse(value: Option<&str>) -> Result<Self> {
        match value {
            None | Some("0") => Ok(Self::Disabled),
            Some("1") => Ok(Self::Enabled),
            Some(_) => bail!("mux must be 0 or 1"),
        }
    }

    pub(crate) const fn enabled(self) -> bool {
        matches!(self, Self::Enabled)
    }
}

impl fmt::Display for MuxMode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(if self.enabled() { "1" } else { "0" })
    }
}

#[cfg(test)]
#[path = "../tests/common/alpn.rs"]
mod tests;
