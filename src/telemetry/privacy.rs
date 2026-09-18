// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

//! Instance-local, category-separated pseudonyms. Keys never leave memory.
use anyhow::Result;
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;

pub(super) fn random_id() -> Result<String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes)
        .map_err(|e| anyhow::anyhow!("telemetry entropy unavailable: {e}"))?;
    Ok(hex(&bytes))
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
pub(super) struct Privacy([u8; 32]);
impl Privacy {
    pub(super) fn new() -> Result<Self> {
        let mut key = [0; 32];
        getrandom::fill(&mut key)
            .map_err(|e| anyhow::anyhow!("telemetry entropy unavailable: {e}"))?;
        Ok(Self(key))
    }
    pub(super) fn alias(&self, category: &str, value: &str) -> String {
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.0).expect("fixed key");
        mac.update(category.as_bytes());
        mac.update(&[0]);
        mac.update(value.as_bytes());
        format!("{category}_{}", hex(&mac.finalize().into_bytes()[..16]))
    }
}
impl Drop for Privacy {
    fn drop(&mut self) {
        for byte in &mut self.0 {
            unsafe {
                std::ptr::write_volatile(byte, 0);
            }
        }
    }
}
#[cfg(test)]
#[path = "../tests/telemetry/privacy.rs"]
mod tests;
