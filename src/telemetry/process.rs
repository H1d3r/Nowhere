// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

//! Portable process identity and best-effort resource sampling.

use std::time::{Instant, SystemTime, UNIX_EPOCH};

use anyhow::Result;

pub(crate) fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}

#[derive(Default)]
pub(super) struct ProcessSample {
    pub(super) cpu_percent: Option<f64>,
    pub(super) rss_bytes: Option<u64>,
    pub(super) open_fds: Option<u64>,
}

#[derive(Default)]
pub(super) struct ProcessSampler {
    previous_ticks: Option<u64>,
    previous_at: Option<Instant>,
}

impl ProcessSampler {
    pub(super) fn sample(&mut self) -> ProcessSample {
        let now = Instant::now();
        let total_ticks = read_process_cpu_ticks();
        let cpu_percent = total_ticks.and_then(|ticks| {
            let result = self.previous_ticks.zip(self.previous_at).and_then(
                |(previous_ticks, previous_at)| {
                    let elapsed = now.saturating_duration_since(previous_at).as_secs_f64();
                    let hz = clock_ticks_per_second();
                    (elapsed > 0.0 && hz > 0).then(|| {
                        ticks.saturating_sub(previous_ticks) as f64 / hz as f64 / elapsed * 100.0
                    })
                },
            );
            self.previous_ticks = Some(ticks);
            self.previous_at = Some(now);
            result
        });
        ProcessSample {
            cpu_percent,
            rss_bytes: read_rss_bytes(),
            open_fds: read_open_fds(),
        }
    }
}

#[cfg(target_os = "linux")]
fn read_process_cpu_ticks() -> Option<u64> {
    let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
    let close = stat.rfind(')')?;
    let fields = stat
        .get(close + 2..)?
        .split_ascii_whitespace()
        .collect::<Vec<_>>();
    let user = fields.get(11)?.parse::<u64>().ok()?;
    let system = fields.get(12)?.parse::<u64>().ok()?;
    Some(user.saturating_add(system))
}

#[cfg(not(target_os = "linux"))]
fn read_process_cpu_ticks() -> Option<u64> {
    None
}

#[cfg(target_os = "linux")]
fn read_rss_bytes() -> Option<u64> {
    let statm = std::fs::read_to_string("/proc/self/statm").ok()?;
    let pages = statm.split_ascii_whitespace().nth(1)?.parse::<u64>().ok()?;
    let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    (page_size > 0).then(|| pages.saturating_mul(page_size as u64))
}

#[cfg(not(target_os = "linux"))]
fn read_rss_bytes() -> Option<u64> {
    None
}

#[cfg(target_os = "linux")]
fn read_open_fds() -> Option<u64> {
    std::fs::read_dir("/proc/self/fd")
        .ok()
        .map(|entries| (entries.count() as u64).saturating_sub(1))
}

#[cfg(not(target_os = "linux"))]
fn read_open_fds() -> Option<u64> {
    None
}

#[cfg(target_os = "linux")]
fn clock_ticks_per_second() -> libc::c_long {
    unsafe { libc::sysconf(libc::_SC_CLK_TCK) }
}

#[cfg(not(target_os = "linux"))]
fn clock_ticks_per_second() -> libc::c_long {
    0
}

pub(super) fn process_uptime_ms(incarnation: u64) -> Option<u64> {
    #[cfg(not(target_os = "linux"))]
    return Some(now_unix_ms().saturating_sub(incarnation));

    #[cfg(target_os = "linux")]
    {
        let system_uptime = std::fs::read_to_string("/proc/uptime")
            .ok()?
            .split_ascii_whitespace()
            .next()?
            .parse::<f64>()
            .ok()?;
        let hz = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
        if hz <= 0 {
            return None;
        }
        let process_started = incarnation as f64 / hz as f64;
        Some(((system_uptime - process_started).max(0.0) * 1_000.0) as u64)
    }
}

pub(super) fn process_uid() -> u32 {
    #[cfg(unix)]
    {
        unsafe { libc::geteuid() }
    }
    #[cfg(not(unix))]
    {
        0
    }
}

pub(super) fn process_incarnation(pid: u32) -> Result<u64> {
    #[cfg(all(not(target_os = "linux"), not(windows)))]
    {
        let _ = pid;
        Ok(now_unix_ms())
    }
    #[cfg(windows)]
    return read_process_incarnation(pid)
        .ok_or_else(|| anyhow::anyhow!("telemetry process identity unavailable"));
    #[cfg(target_os = "linux")]
    read_process_incarnation(pid)
        .ok_or_else(|| anyhow::anyhow!("telemetry: failed to read /proc/{pid}/stat start time"))
}

pub(crate) fn read_process_incarnation(pid: u32) -> Option<u64> {
    #[cfg(windows)]
    return windows_creation(pid);
    #[cfg(all(not(target_os = "linux"), not(windows)))]
    {
        let _ = pid;
        None
    }
    #[cfg(target_os = "linux")]
    {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        let close = stat.rfind(')')?;
        // The suffix starts at field 3 (`state`); starttime is field 22.
        stat.get(close + 2..)?
            .split_ascii_whitespace()
            .nth(19)?
            .parse()
            .ok()
    }
}

pub(crate) fn process_is_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        let result = unsafe { libc::kill(pid as libc::pid_t, 0) };
        // Only a confirmed missing process permits registry cleanup.
        result == 0 || std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::{Foundation::*, System::Threading::*};
        let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if process.is_null() {
            return std::io::Error::last_os_error().raw_os_error()
                != Some(ERROR_INVALID_PARAMETER as i32);
        }
        let mut code = 0;
        let ok = unsafe { GetExitCodeProcess(process, &mut code) };
        unsafe {
            CloseHandle(process);
        }
        ok == 0 || code == 259
    }
}

#[cfg(windows)]
fn windows_creation(pid: u32) -> Option<u64> {
    use windows_sys::Win32::{Foundation::*, System::Threading::*};
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        return None;
    }
    let mut created = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let mut exit = created;
    let mut kernel = created;
    let mut user = created;
    let ok = unsafe { GetProcessTimes(process, &mut created, &mut exit, &mut kernel, &mut user) };
    unsafe {
        CloseHandle(process);
    }
    if ok == 0 {
        return None;
    }
    let ticks = (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime);
    Some(ticks.checked_sub(116_444_736_000_000_000)? / 10_000)
}
