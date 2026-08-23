// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

use super::*;

#[test]
fn latest_live_sample_wins_and_drop_restores_the_previous_one() {
    let tracker = LatencyTracker::new();
    let first = tracker.register();
    first.update(Duration::from_millis(12));
    assert_eq!(tracker.current_ms(), 12);

    let second = tracker.register();
    second.update(Duration::from_millis(7));
    assert_eq!(tracker.current_ms(), 7);

    drop(second);
    assert_eq!(tracker.current_ms(), 12);
    drop(first);
    assert_eq!(tracker.current_ms(), 0);
}

#[test]
fn valid_sub_millisecond_samples_round_up_away_from_zero() {
    let tracker = LatencyTracker::new();
    let sample = tracker.register();
    sample.update(Duration::from_micros(1));
    assert_eq!(tracker.current_ms(), 1);
    sample.update(Duration::from_micros(1_001));
    assert_eq!(tracker.current_ms(), 2);
}

#[cfg(target_os = "linux")]
#[test]
fn tcp_info_accepts_a_kernel_prefix_containing_rtt() {
    // SAFETY: tcp_info contains only integer fields, so zero is valid.
    let mut info = unsafe { MaybeUninit::<libc::tcp_info>::zeroed().assume_init() };
    info.tcpi_rtt = 750;
    let rtt_end = std::mem::offset_of!(libc::tcp_info, tcpi_rtt) + size_of::<u32>();

    assert_eq!(
        tcp_info_rtt(&info, rtt_end),
        Some(Duration::from_micros(750))
    );
    assert_eq!(tcp_info_rtt(&info, rtt_end - 1), None);
}
