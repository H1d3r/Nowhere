use super::*;

fn meta(id: &str, pid: u32) -> InstanceMeta {
    InstanceMeta {
        id: id.to_owned(),
        role: InstanceRole::Portal,
        pid,
        uid: 0,
        endpoint: format!(":{pid}"),
        telemetry_interval_ms: 1_000,
        ..InstanceMeta::default()
    }
}

fn snapshot(at: u64, up: u64, down: u64) -> TelemetrySnapshot {
    TelemetrySnapshot {
        timestamp_ms: at,
        tcp_rx: up,
        tcp_tx: down,
        ..TelemetrySnapshot::default()
    }
}

#[test]
fn calculates_bits_per_second_and_trims_history() {
    let mut app = App::default();
    app.apply(UiEvent::Upsert {
        meta: meta("one", 1),
        lifecycle: Lifecycle::Ready,
        snapshot: Some(snapshot(1_000, 100, 200)),
    });
    app.apply(UiEvent::Snapshot {
        id: "one".to_owned(),
        snapshot: snapshot(2_000, 1_100, 2_200),
    });
    let point = app.selected().unwrap().latest_history();
    assert_eq!(point.upload_bps, 8_000.0);
    assert_eq!(point.download_bps, 16_000.0);

    app.apply(UiEvent::Snapshot {
        id: "one".to_owned(),
        snapshot: snapshot(HISTORY_WINDOW_MS + 3_000, 2_100, 3_200),
    });
    assert_eq!(app.selected().unwrap().history.len(), 1);
}

#[test]
fn retains_ping_in_the_ten_minute_history() {
    let mut app = App::default();
    let mut first = snapshot(1_000, 0, 0);
    first.ping_ms = 7;
    app.apply(UiEvent::Upsert {
        meta: meta("one", 1),
        lifecycle: Lifecycle::Ready,
        snapshot: Some(first),
    });
    let mut second = snapshot(2_000, 0, 0);
    second.ping_ms = 11;
    app.apply(UiEvent::Snapshot {
        id: "one".to_owned(),
        snapshot: second,
    });

    assert_eq!(app.selected().unwrap().latest_history().ping_ms, 11);
}

#[test]
fn counter_reset_clears_history_without_negative_rate() {
    let mut view = InstanceView::new(
        meta("one", 1),
        Lifecycle::Ready,
        Some(snapshot(1_000, 1_000, 2_000)),
    );
    view.update_snapshot(snapshot(2_000, 2_000, 4_000));
    assert_eq!(view.history.len(), 1);
    view.update_snapshot(snapshot(3_000, 3, 4));
    assert!(view.history.is_empty());
}

#[test]
fn rate_uses_monotonic_uptime_when_wall_clock_moves() {
    let mut first = snapshot(10_000, 100, 0);
    first.uptime_ms = 1_000;
    let mut view = InstanceView::new(meta("one", 1), Lifecycle::Ready, Some(first));
    let mut second = snapshot(500, 1_100, 0);
    second.uptime_ms = 2_000;
    view.update_snapshot(second);
    assert_eq!(view.latest_history().upload_bps, 8_000.0);
}

#[test]
fn feed_buffers_are_bounded() {
    let mut view = InstanceView::new(meta("one", 1), Lifecycle::Ready, None);
    for event_id in 0..FEED_CAPACITY + 5 {
        view.push_access(AccessRecord {
            event_id: event_id as u64,
            ..AccessRecord::default()
        });
    }
    assert_eq!(view.access.len(), FEED_CAPACITY);
    assert_eq!(view.access.front().unwrap().event_id, 5);
    assert_eq!(view.overwritten_events, 5);
}

#[test]
fn completion_replaces_its_live_access_row() {
    let mut view = InstanceView::new(meta("one", 1), Lifecycle::Ready, None);
    assert!(view.push_access(AccessRecord {
        event_id: 7,
        phase: AccessPhase::Start,
        ..AccessRecord::default()
    }));
    assert!(!view.push_access(AccessRecord {
        event_id: 7,
        phase: AccessPhase::Finish,
        status: Some(AccessStatus::Success),
        ..AccessRecord::default()
    }));
    assert_eq!(view.access.len(), 1);
    assert_eq!(view.access[0].status, Some(AccessStatus::Success));
}

#[test]
fn selection_survives_sorting_and_offline_instances_expire() {
    let mut app = App::default();
    app.apply(UiEvent::Upsert {
        meta: meta("second", 2),
        lifecycle: Lifecycle::Ready,
        snapshot: None,
    });
    app.apply(UiEvent::Upsert {
        meta: meta("first", 1),
        lifecycle: Lifecycle::Ready,
        snapshot: None,
    });
    assert_eq!(app.selected().unwrap().meta.id, "second");
    app.apply(UiEvent::Offline {
        id: "second".to_owned(),
    });
    let future = Instant::now() + OFFLINE_RETENTION + Duration::from_secs(1);
    app.tick(future);
    assert_eq!(app.selected().unwrap().meta.id, "first");
}

#[test]
fn paused_filtered_feed_only_tracks_matching_new_records() {
    let mut app = App::default();
    app.apply(UiEvent::Upsert {
        meta: meta("one", 1),
        lifecycle: Lifecycle::Ready,
        snapshot: None,
    });
    app.feed = FeedKind::Runtime;
    app.paused = true;
    app.filter = "carrier".to_owned();
    app.apply(UiEvent::Runtime {
        id: "one".to_owned(),
        record: RuntimeRecord {
            kind: "POOL".to_owned(),
            ..RuntimeRecord::default()
        },
    });
    assert_eq!(app.feed_scroll, 0);
    app.apply(UiEvent::Runtime {
        id: "one".to_owned(),
        record: RuntimeRecord {
            kind: "CARRIER".to_owned(),
            ..RuntimeRecord::default()
        },
    });
    assert_eq!(app.feed_scroll, 1);
}
