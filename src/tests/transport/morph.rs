// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

//! Module wiring for Morph key, TCP, and UDP tests.

use std::io;

use chacha20::ChaCha20;
use chacha20::cipher::{KeyIvInit, StreamCipher, StreamCipherSeek};

use super::tcp::MorphTcpStream;
use super::udp::{MorphUdpSocket, UdpReceiveState, UdpSendState};
use super::*;

fn apply_at(
    key: &MorphKeyBytes,
    nonce: &[u8; NONCE_LEN],
    offset: u64,
    bytes: &mut [u8],
) -> io::Result<()> {
    let mut cipher = ChaCha20::new(key.into(), nonce.into());
    cipher.try_seek(offset).map_err(|_| exhausted())?;
    cipher.try_apply_keystream(bytes).map_err(|_| exhausted())
}

#[path = "morph/keys.rs"]
mod keys;
#[path = "morph/tcp.rs"]
mod tcp;
#[path = "morph/udp.rs"]
mod udp;
