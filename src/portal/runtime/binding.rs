// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

use super::*;

impl Portal {
    /// Returns the effective startup URL that is logged for operators.
    pub(in crate::portal) fn effective_url(&self) -> String {
        let base = format!(
            "portal://{}?tls={}&rate={}&etar={}&dial={}&morph={}&socks={}&next={}",
            self.inner.endpoint_addr,
            self.inner.tls_mode,
            self.inner.rate_limit,
            self.inner.etar_limit,
            self.inner.outbound.dialer_ip(),
            u8::from(self.inner.morph_keys.is_some()),
            self.inner.outbound.socks_endpoint(),
            self.inner.outbound.next_endpoint(),
        );
        self.inner
            .outbound
            .next_transport()
            .map_or(base.clone(), |transport| {
                let upstream = transport
                    .split_whitespace()
                    .filter(|option| !option.starts_with("morph="))
                    .collect::<Vec<_>>()
                    .join("&");
                format!("{base}&{upstream}")
            })
    }

    /// Opens QUIC endpoints for network modes that accept UDP service.
    pub(in crate::portal) fn listen_endpoints(&self) -> Result<Vec<Endpoint>> {
        if !self.inner.network_mode.listens_udp() {
            return Ok(Vec::new());
        }
        bind_carrier(
            &self.inner.udp_bind_addrs,
            self.inner.allow_udp_family_degrade,
            |addr| listen_endpoint(
                self.inner.quic_server_config.clone(),
                addr,
                self.inner.morph_keys.clone(),
            ),
            |addr, error| self.inner.logger.warn(format_args!(
                "portal::listen_endpoints: UDP address family unavailable for {addr}; continuing: {error:#}"
            )),
        ).context("portal::listen_endpoints: failed to open UDP listeners")
    }

    /// Opens TLS/TCP listeners for network modes that accept TCP service.
    pub(in crate::portal) fn listen_tcp_listeners(&self) -> Result<Vec<TcpListener>> {
        if !self.inner.network_mode.listens_tcp() {
            return Ok(Vec::new());
        }
        bind_carrier(
            &self.inner.tcp_bind_addrs,
            self.inner.allow_tcp_family_degrade,
            listen_tcp,
            |addr, error| self.inner.logger.warn(format_args!(
                "portal::listen_tcp_listeners: TCP address family unavailable for {addr}; continuing: {error:#}"
            )),
        ).context("portal::listen_tcp_listeners: failed to open TCP listeners")
    }
}

/// Owns every successful bind until the whole carrier has passed validation.
/// Owns every successful bind until the whole carrier has passed validation.
pub(super) fn bind_carrier<T>(
    addresses: &[std::net::SocketAddr],
    allow_degrade: bool,
    mut bind: impl FnMut(std::net::SocketAddr) -> Result<T>,
    mut warn: impl FnMut(std::net::SocketAddr, &anyhow::Error),
) -> Result<Vec<T>> {
    let mut listeners = Vec::new();
    for &address in addresses {
        match bind(address) {
            Ok(listener) => listeners.push(listener),
            Err(error) if allow_degrade && family_is_unavailable(&error) => warn(address, &error),
            Err(error) => return Err(error),
        }
    }
    if listeners.is_empty() {
        anyhow::bail!("no declared address could be bound");
    }
    Ok(listeners)
}

fn family_is_unavailable(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause
            .downcast_ref::<std::io::Error>()
            .is_some_and(io_error_is_family_unavailable)
    })
}

pub(super) fn io_error_is_family_unavailable(error: &std::io::Error) -> bool {
    #[cfg(unix)]
    const FAMILY_UNAVAILABLE: i32 = libc::EAFNOSUPPORT;
    #[cfg(windows)]
    const FAMILY_UNAVAILABLE: i32 = 10047; // WSAEAFNOSUPPORT
    matches!(
        error.kind(),
        std::io::ErrorKind::AddrNotAvailable | std::io::ErrorKind::Unsupported
    ) || error.raw_os_error() == Some(FAMILY_UNAVAILABLE)
}
