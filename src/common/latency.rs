// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

//! Allocation-free reads of the latest active upstream transport RTT.

use std::collections::HashMap;
use std::mem::{MaybeUninit, size_of};
use std::os::fd::AsRawFd;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

#[derive(Clone, Copy, Debug)]
struct Sample {
    milliseconds: u64,
    sequence: u64,
}

/// Tracks only live upstream carriers. Updates happen at carrier boundaries,
/// never on the payload hot path; readers use one relaxed atomic load.
#[derive(Debug)]
pub(crate) struct LatencyTracker {
    current_ms: AtomicU64,
    next_id: AtomicU64,
    next_sequence: AtomicU64,
    samples: Mutex<HashMap<u64, Sample>>,
}

impl LatencyTracker {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            current_ms: AtomicU64::new(0),
            next_id: AtomicU64::new(1),
            next_sequence: AtomicU64::new(1),
            samples: Mutex::new(HashMap::new()),
        })
    }

    pub(crate) fn register(self: &Arc<Self>) -> LatencyGuard {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.samples
            .lock()
            .unwrap_or_else(|lock| lock.into_inner())
            .insert(
                id,
                Sample {
                    milliseconds: 0,
                    sequence: 0,
                },
            );
        LatencyGuard {
            tracker: Arc::downgrade(self),
            id,
        }
    }

    pub(crate) fn current_ms(&self) -> u64 {
        self.current_ms.load(Ordering::Relaxed)
    }
}

/// Keeps one RTT sample live for exactly as long as its physical carrier.
pub(crate) struct LatencyGuard {
    tracker: Weak<LatencyTracker>,
    id: u64,
}

impl LatencyGuard {
    pub(crate) fn update(&self, duration: Duration) {
        let Some(tracker) = self.tracker.upgrade() else {
            return;
        };
        let milliseconds = duration
            .as_micros()
            .saturating_add(999)
            .checked_div(1_000)
            .unwrap_or_default()
            .clamp(1, u64::MAX as u128) as u64;
        let sequence = tracker.next_sequence.fetch_add(1, Ordering::Relaxed);
        let mut samples = tracker
            .samples
            .lock()
            .unwrap_or_else(|lock| lock.into_inner());
        let Some(sample) = samples.get_mut(&self.id) else {
            return;
        };
        *sample = Sample {
            milliseconds,
            sequence,
        };
        tracker.current_ms.store(milliseconds, Ordering::Relaxed);
    }

    pub(crate) fn update_tcp<T: AsRawFd>(&self, stream: &T) {
        if let Some(duration) = tcp_rtt(stream) {
            self.update(duration);
        }
    }
}

impl Drop for LatencyGuard {
    fn drop(&mut self) {
        let Some(tracker) = self.tracker.upgrade() else {
            return;
        };
        let mut samples = tracker
            .samples
            .lock()
            .unwrap_or_else(|lock| lock.into_inner());
        samples.remove(&self.id);
        let fallback = samples
            .values()
            .filter(|sample| sample.milliseconds != 0)
            .max_by_key(|sample| sample.sequence)
            .map_or(0, |sample| sample.milliseconds);
        tracker.current_ms.store(fallback, Ordering::Relaxed);
    }
}

fn tcp_rtt<T: AsRawFd>(stream: &T) -> Option<Duration> {
    let mut info = MaybeUninit::<libc::tcp_info>::zeroed();
    let mut length = size_of::<libc::tcp_info>() as libc::socklen_t;
    // SAFETY: TCP_INFO writes at most `length` bytes to a correctly sized,
    // aligned tcp_info buffer. The descriptor remains owned by the caller.
    let result = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::IPPROTO_TCP,
            libc::TCP_INFO,
            info.as_mut_ptr().cast(),
            &mut length,
        )
    };
    if result != 0 || length < size_of::<libc::tcp_info>() as libc::socklen_t {
        return None;
    }
    // SAFETY: getsockopt succeeded and initialized the full structure.
    let micros = unsafe { info.assume_init() }.tcpi_rtt;
    Some(Duration::from_micros(u64::from(micros)))
}

#[cfg(test)]
#[path = "../tests/common/latency.rs"]
mod tests;
