// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

//! TUI-owned view model.
//!
//! The IPC protocol deliberately does not leak into the renderer.  The client
//! adapter normalizes wire messages into [`UiEvent`] values and this module
//! keeps the short, process-local history needed by charts and feeds.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

mod filter;
mod metrics;
mod types;

pub use filter::{access_matches, runtime_matches};
pub use metrics::{HISTORY_WINDOW_MS, HistoryPoint, TelemetrySnapshot};
pub use types::{
    AccessPhase, AccessRecord, AccessStatus, EventLevel, FeedKind, Focus, InstanceId, InstanceMeta,
    InstanceRole, Lifecycle, Page, RuntimeRecord, UiEvent,
};

/// Maximum number of access or runtime records kept by one TUI.
pub const FEED_CAPACITY: usize = 2_000;
const OFFLINE_RETENTION: Duration = Duration::from_secs(30);

/// One process and all state retained locally for it.
#[derive(Clone, Debug)]
pub struct InstanceView {
    pub meta: InstanceMeta,
    pub lifecycle: Lifecycle,
    pub online: bool,
    pub snapshot: Option<TelemetrySnapshot>,
    pub history: VecDeque<HistoryPoint>,
    pub access: VecDeque<AccessRecord>,
    pub runtime: VecDeque<RuntimeRecord>,
    pub dropped_events: u64,
    pub overwritten_events: u64,
    offline_since: Option<Instant>,
}

impl InstanceView {
    fn new(meta: InstanceMeta, lifecycle: Lifecycle, snapshot: Option<TelemetrySnapshot>) -> Self {
        Self {
            meta,
            lifecycle,
            online: true,
            snapshot,
            history: VecDeque::new(),
            access: VecDeque::with_capacity(FEED_CAPACITY),
            runtime: VecDeque::with_capacity(FEED_CAPACITY),
            dropped_events: 0,
            overwritten_events: 0,
            offline_since: None,
        }
    }

    fn update_snapshot(&mut self, snapshot: TelemetrySnapshot) {
        if let Some(previous) = self.snapshot.as_ref() {
            let sample_clock = snapshot.sample_clock_ms();
            let previous_clock = previous.sample_clock_ms();
            let elapsed_ms = sample_clock.saturating_sub(previous_clock);
            if elapsed_ms == 0 || snapshot.counter_reset_from(previous) {
                if sample_clock < previous_clock || snapshot.counter_reset_from(previous) {
                    self.history.clear();
                }
            } else {
                let rate = |new: u64, old: u64| {
                    new.saturating_sub(old) as f64 * 8_000.0 / elapsed_ms as f64
                };
                self.history.push_back(HistoryPoint {
                    timestamp_ms: sample_clock,
                    upload_bps: rate(snapshot.upload_bytes(), previous.upload_bytes()),
                    download_bps: rate(snapshot.download_bytes(), previous.download_bytes()),
                    tcp_bps: rate(snapshot.tcp_bytes(), previous.tcp_bytes()),
                    udp_bps: rate(snapshot.udp_bytes(), previous.udp_bytes()),
                    tls_bps: rate(snapshot.tls_bytes(), previous.tls_bytes()),
                    quic_bps: rate(snapshot.quic_bytes(), previous.quic_bytes()),
                    tcp_active: snapshot.tcp_active.max(0),
                    udp_active: snapshot.udp_active.max(0),
                    tls_links: snapshot.tls_carriers_active,
                    quic_links: snapshot.quic_carriers_active,
                    cpu_percent: snapshot.cpu_percent.unwrap_or_default().max(0.0),
                    rss_bytes: snapshot.rss_bytes.unwrap_or_default(),
                });
                while self.history.front().is_some_and(|point| {
                    sample_clock.saturating_sub(point.timestamp_ms) > HISTORY_WINDOW_MS
                }) {
                    self.history.pop_front();
                }
            }
        }
        self.online = true;
        self.offline_since = None;
        self.snapshot = Some(snapshot);
    }

    fn push_access(&mut self, record: AccessRecord) -> bool {
        if record.phase == AccessPhase::Finish
            && let Some(existing) = self
                .access
                .iter_mut()
                .rev()
                .find(|existing| existing.event_id == record.event_id)
        {
            *existing = record;
            return false;
        }
        if push_bounded(&mut self.access, record) {
            self.overwritten_events = self.overwritten_events.saturating_add(1);
        }
        true
    }

    fn push_runtime(&mut self, record: RuntimeRecord) {
        if push_bounded(&mut self.runtime, record) {
            self.overwritten_events = self.overwritten_events.saturating_add(1);
        }
    }

    fn mark_offline(&mut self, now: Instant) {
        self.online = false;
        self.offline_since.get_or_insert(now);
    }

    fn expired(&self, now: Instant) -> bool {
        self.offline_since
            .is_some_and(|since| now.saturating_duration_since(since) >= OFFLINE_RETENTION)
    }

    pub fn latest_history(&self) -> HistoryPoint {
        self.history.back().copied().unwrap_or_default()
    }
}

fn push_bounded<T>(queue: &mut VecDeque<T>, value: T) -> bool {
    let overwritten = queue.len() == FEED_CAPACITY;
    if queue.len() == FEED_CAPACITY {
        queue.pop_front();
    }
    queue.push_back(value);
    overwritten
}

/// Terminal features selected once at startup.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Capabilities {
    pub unicode: bool,
    pub color: bool,
}

impl Default for Capabilities {
    fn default() -> Self {
        Self {
            unicode: true,
            color: true,
        }
    }
}

/// Complete local state for a TUI process.
#[derive(Debug, Default)]
pub struct App {
    pub instances: Vec<InstanceView>,
    selected_id: Option<InstanceId>,
    pub focus: Focus,
    pub page: Page,
    pub feed: FeedKind,
    pub paused: bool,
    pub feed_scroll: usize,
    pub access_horizontal_scroll: usize,
    pub runtime_horizontal_scroll: usize,
    pub filter: String,
    pub filter_editing: bool,
    pub reveal_clients: bool,
    pub show_help: bool,
    pub should_quit: bool,
    pub global_error: Option<String>,
    pub capabilities: Capabilities,
}

impl App {
    pub fn apply(&mut self, event: UiEvent) {
        match event {
            UiEvent::Upsert {
                meta,
                lifecycle,
                snapshot,
            } => {
                self.global_error = None;
                if let Some(instance) = self
                    .instances
                    .iter_mut()
                    .find(|instance| instance.meta.id == meta.id)
                {
                    instance.meta = meta;
                    instance.lifecycle = lifecycle;
                    instance.online = true;
                    instance.offline_since = None;
                    if let Some(snapshot) = snapshot {
                        instance.update_snapshot(snapshot);
                    }
                } else {
                    let id = meta.id.clone();
                    self.instances
                        .push(InstanceView::new(meta, lifecycle, snapshot));
                    if self.selected_id.is_none() {
                        self.selected_id = Some(id);
                    }
                }
                self.sort_instances();
            }
            UiEvent::Snapshot { id, snapshot } => {
                if let Some(instance) = self.find_mut(&id) {
                    instance.update_snapshot(snapshot);
                }
            }
            UiEvent::Lifecycle { id, lifecycle } => {
                if let Some(instance) = self.find_mut(&id) {
                    instance.lifecycle = lifecycle;
                }
            }
            UiEvent::Runtime { id, record } => {
                let preserve_scroll = self.selected_id.as_deref() == Some(id.as_str())
                    && self.paused
                    && self.feed == FeedKind::Runtime
                    && runtime_matches(&record, &self.filter.to_ascii_lowercase());
                if let Some(instance) = self.find_mut(&id) {
                    instance.push_runtime(record);
                    if preserve_scroll {
                        self.feed_scroll = self.feed_scroll.saturating_add(1);
                    }
                }
            }
            UiEvent::Access { id, record } => {
                let preserve_scroll = self.selected_id.as_deref() == Some(id.as_str())
                    && self.paused
                    && self.feed == FeedKind::Access
                    && access_matches(&record, &self.filter.to_ascii_lowercase());
                if let Some(instance) = self.find_mut(&id) {
                    let appended = instance.push_access(record);
                    if preserve_scroll && appended {
                        self.feed_scroll = self.feed_scroll.saturating_add(1);
                    }
                }
            }
            UiEvent::Gap { id, missed } => {
                if let Some(instance) = self.find_mut(&id) {
                    instance.dropped_events = instance.dropped_events.saturating_add(missed);
                }
            }
            UiEvent::Offline { id } => {
                if let Some(instance) = self.find_mut(&id) {
                    instance.mark_offline(Instant::now());
                }
            }
            UiEvent::Error { id, message } => {
                if let Some(id) = id {
                    if let Some(instance) = self.find_mut(&id) {
                        instance.push_runtime(RuntimeRecord {
                            level: EventLevel::Error,
                            kind: "IPC".to_owned(),
                            message,
                            ..RuntimeRecord::default()
                        });
                    }
                } else {
                    self.global_error = Some(message);
                }
            }
        }
    }

    pub fn tick(&mut self, now: Instant) {
        let old_selected = self.selected_id.clone();
        self.instances.retain(|instance| !instance.expired(now));
        if old_selected
            .as_ref()
            .is_some_and(|id| !self.instances.iter().any(|item| &item.meta.id == id))
        {
            self.selected_id = self
                .instances
                .first()
                .map(|instance| instance.meta.id.clone());
            self.feed_scroll = 0;
            self.reset_horizontal_scroll();
        }
    }

    pub fn selected(&self) -> Option<&InstanceView> {
        let id = self.selected_id.as_deref()?;
        self.instances
            .iter()
            .find(|instance| instance.meta.id == id)
    }

    pub fn selected_id(&self) -> Option<&str> {
        self.selected_id.as_deref()
    }

    pub fn selected_mut(&mut self) -> Option<&mut InstanceView> {
        let id = self.selected_id.clone()?;
        self.instances
            .iter_mut()
            .find(|instance| instance.meta.id == id)
    }

    pub fn selected_index(&self) -> Option<usize> {
        let id = self.selected_id.as_deref()?;
        self.instances
            .iter()
            .position(|instance| instance.meta.id == id)
    }

    pub fn select_relative(&mut self, delta: isize) {
        if self.instances.is_empty() {
            self.selected_id = None;
            return;
        }
        let current = self.selected_index().unwrap_or_default() as isize;
        let last = self.instances.len().saturating_sub(1) as isize;
        let next = current.saturating_add(delta).clamp(0, last) as usize;
        if self.selected_index() != Some(next) {
            self.selected_id = Some(self.instances[next].meta.id.clone());
            self.feed_scroll = 0;
            self.reset_horizontal_scroll();
            self.paused = false;
        }
    }

    pub fn select_first(&mut self) {
        if let Some(instance) = self.instances.first() {
            self.selected_id = Some(instance.meta.id.clone());
            self.feed_scroll = 0;
            self.reset_horizontal_scroll();
        }
    }

    pub fn select_last(&mut self) {
        if let Some(instance) = self.instances.last() {
            self.selected_id = Some(instance.meta.id.clone());
            self.feed_scroll = 0;
            self.reset_horizontal_scroll();
        }
    }

    pub fn clear_current_feed(&mut self) {
        let feed = self.feed;
        if let Some(instance) = self.selected_mut() {
            match feed {
                FeedKind::Access => instance.access.clear(),
                FeedKind::Runtime => instance.runtime.clear(),
            }
        }
        self.feed_scroll = 0;
        self.set_feed_horizontal_scroll(0);
    }

    pub fn set_feed(&mut self, feed: FeedKind) {
        self.feed = feed;
        self.focus = Focus::Feed;
        self.feed_scroll = 0;
        self.page = Page::Logs;
    }

    pub fn focus_next(&mut self) {
        match (self.page, self.focus, self.feed) {
            (Page::Overview, _, _) | (Page::Logs, Focus::Instances, _) => {
                self.set_feed(FeedKind::Access);
            }
            (Page::Logs, Focus::Feed, FeedKind::Access) => {
                self.set_feed(FeedKind::Runtime);
            }
            (Page::Logs, Focus::Feed, FeedKind::Runtime) => {
                self.focus = Focus::Instances;
            }
        }
    }

    pub fn focus_previous(&mut self) {
        match (self.page, self.focus, self.feed) {
            (Page::Overview, _, _) | (Page::Logs, Focus::Instances, _) => {
                self.set_feed(FeedKind::Runtime);
            }
            (Page::Logs, Focus::Feed, FeedKind::Access) => {
                self.focus = Focus::Instances;
            }
            (Page::Logs, Focus::Feed, FeedKind::Runtime) => self.set_feed(FeedKind::Access),
        }
    }

    pub const fn feed_horizontal_scroll(&self, feed: FeedKind) -> usize {
        match feed {
            FeedKind::Access => self.access_horizontal_scroll,
            FeedKind::Runtime => self.runtime_horizontal_scroll,
        }
    }

    pub fn scroll_feed_horizontal(&mut self, delta: isize) {
        const MAX_HORIZONTAL_SCROLL: usize = 4_096;
        let next = self
            .feed_horizontal_scroll(self.feed)
            .saturating_add_signed(delta)
            .min(MAX_HORIZONTAL_SCROLL);
        self.set_feed_horizontal_scroll(next);
    }

    pub fn reset_horizontal_scroll(&mut self) {
        self.access_horizontal_scroll = 0;
        self.runtime_horizontal_scroll = 0;
    }

    fn set_feed_horizontal_scroll(&mut self, value: usize) {
        match self.feed {
            FeedKind::Access => self.access_horizontal_scroll = value,
            FeedKind::Runtime => self.runtime_horizontal_scroll = value,
        }
    }

    pub fn scroll_feed(&mut self, delta: isize) {
        let max = self.filtered_feed_len().saturating_sub(1);
        self.feed_scroll = self.feed_scroll.saturating_add_signed(delta).min(max);
        self.paused = self.feed_scroll != 0;
    }

    pub fn filtered_feed_len(&self) -> usize {
        let Some(instance) = self.selected() else {
            return 0;
        };
        let filter = self.filter.to_ascii_lowercase();
        match self.feed {
            FeedKind::Access => instance
                .access
                .iter()
                .filter(|record| access_matches(record, &filter))
                .count(),
            FeedKind::Runtime => instance
                .runtime
                .iter()
                .filter(|record| runtime_matches(record, &filter))
                .count(),
        }
    }

    fn find_mut(&mut self, id: &str) -> Option<&mut InstanceView> {
        self.instances
            .iter_mut()
            .find(|instance| instance.meta.id == id)
    }

    fn sort_instances(&mut self) {
        self.instances.sort_by(|a, b| {
            (
                a.meta.uid,
                a.meta.role,
                a.meta.endpoint.as_str(),
                a.meta.pid,
            )
                .cmp(&(
                    b.meta.uid,
                    b.meta.role,
                    b.meta.endpoint.as_str(),
                    b.meta.pid,
                ))
        });
    }
}

#[cfg(test)]
#[path = "../tests/tui/model.rs"]
mod tests;
