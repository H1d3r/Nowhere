// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

//! Compact, allocation-light display formatting.

use std::time::Duration;

use chrono::{Local, TimeZone};

pub fn bits_per_second(value: f64) -> String {
    if !value.is_finite() || value <= 0.0 {
        return "0 b/s".to_owned();
    }
    const UNITS: [(&str, f64); 4] = [
        ("Tb/s", 1_000_000_000_000.0),
        ("Gb/s", 1_000_000_000.0),
        ("Mb/s", 1_000_000.0),
        ("Kb/s", 1_000.0),
    ];
    for (unit, divisor) in UNITS {
        if value >= divisor {
            let scaled = value / divisor;
            return if scaled >= 100.0 {
                format!("{scaled:.0} {unit}")
            } else if scaled >= 10.0 {
                format!("{scaled:.1} {unit}")
            } else {
                format!("{scaled:.2} {unit}")
            };
        }
    }
    format!("{value:.0} b/s")
}

pub fn bytes(value: u64) -> String {
    const UNITS: [(&str, u64); 6] = [
        ("PiB", 1 << 50),
        ("TiB", 1 << 40),
        ("GiB", 1 << 30),
        ("MiB", 1 << 20),
        ("KiB", 1 << 10),
        ("B", 1),
    ];
    for (unit, divisor) in UNITS {
        if value >= divisor {
            if divisor == 1 {
                return format!("{value} B");
            }
            let scaled = value as f64 / divisor as f64;
            return if scaled >= 100.0 {
                format!("{scaled:.0} {unit}")
            } else if scaled >= 10.0 {
                format!("{scaled:.1} {unit}")
            } else {
                format!("{scaled:.2} {unit}")
            };
        }
    }
    "0 B".to_owned()
}

pub fn duration_ms(value: u64) -> String {
    if value < 1_000 {
        return format!("{value}ms");
    }
    let duration = Duration::from_millis(value);
    let seconds = duration.as_secs();
    if seconds < 60 {
        return format!("{:.1}s", value as f64 / 1_000.0);
    }
    let minutes = seconds / 60;
    let remaining_seconds = seconds % 60;
    if minutes < 60 {
        return format!("{minutes}m {remaining_seconds:02}s");
    }
    let hours = minutes / 60;
    let remaining_minutes = minutes % 60;
    if hours < 24 {
        return format!("{hours}h {remaining_minutes:02}m");
    }
    let days = hours / 24;
    format!("{days}d {:02}h", hours % 24)
}

pub fn clock_time(timestamp_ms: u64) -> String {
    i64::try_from(timestamp_ms)
        .ok()
        .and_then(|timestamp| Local.timestamp_millis_opt(timestamp).single())
        .map(|time| time.format("%H:%M:%S").to_string())
        .unwrap_or_else(|| "--:--:--".to_owned())
}

/// Only instance-local pseudonyms may be rendered as client identities.
pub fn client_address(value: &str) -> String {
    if value.starts_with('C')
        && (4..=21).contains(&value.len())
        && value[1..].bytes().all(|b| b.is_ascii_digit())
        && value[1..].parse::<u64>().is_ok_and(|id| id > 0)
    {
        value.to_owned()
    } else {
        "<redacted>".to_owned()
    }
}

pub fn truncate(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_owned();
    }
    if max_chars == 0 {
        return String::new();
    }
    if max_chars == 1 {
        return "…".to_owned();
    }
    let mut result: String = value.chars().take(max_chars - 1).collect();
    result.push('…');
    result
}

#[cfg(test)]
#[path = "../tests/tui/format.rs"]
mod tests;
