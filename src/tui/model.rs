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
mod instance;
mod metrics;
mod types;

pub use filter::{access_matches, runtime_matches};
pub use instance::InstanceView;
#[cfg(test)]
use instance::OFFLINE_RETENTION;
pub use metrics::{HISTORY_WINDOW_MS, HistoryPoint, TelemetrySnapshot};
pub use types::{
    AccessPhase, AccessRecord, AccessStatus, EventLevel, FeedKind, Focus, InstanceId, InstanceMeta,
    InstanceRole, Lifecycle, Page, RuntimeRecord, UiEvent,
};

/// Maximum number of access or runtime records kept by one TUI.
pub const FEED_CAPACITY: usize = 2_000;
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
    pub show_help: bool,
    pub show_config: bool,
    pub config_scroll: usize,
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
            self.show_config = false;
            self.config_scroll = 0;
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
