// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

//! Morph TCP constructors used by internal tests.

use super::*;

impl<S: AsyncRead + AsyncWrite + Unpin> MorphTcpStream<S> {
    pub(in crate::transport::morph) fn initialized_for_test(
        inner: S,
        keys: &MorphKeys,
        nonce: [u8; NONCE_LEN],
        client: bool,
    ) -> Self {
        Self::initialized(inner, keys, nonce, client)
    }

    pub(in crate::transport::morph) async fn connect_for_test(
        inner: S,
        keys: MorphKeys,
        prelude: [u8; TCP_PRELUDE_LEN],
        nonce: [u8; NONCE_LEN],
    ) -> io::Result<Self> {
        Self::connect_with_bootstrap(inner, keys, prelude, nonce).await
    }
}
