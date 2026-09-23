// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

//! Shared imports and module wiring for Mux runtime tests.

use super::wire::{CLOSE_FIN, CLOSE_RESET, FrameHeader, encode_header};
use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[path = "runtime/admission.rs"]
mod admission;
#[path = "runtime/streams.rs"]
mod streams;
#[path = "runtime/wire_and_lifecycle.rs"]
mod wire_and_lifecycle;
