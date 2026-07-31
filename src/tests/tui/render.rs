use std::collections::VecDeque;

use ratatui::Terminal;
use ratatui::backend::TestBackend;

use super::*;
use crate::tui::model::{
    AccessPhase, AccessRecord, AccessStatus, EventLevel, HistoryPoint, InstanceMeta, InstanceRole,
    Lifecycle, RuntimeRecord, TelemetrySnapshot, UiEvent,
};

fn app_with_instance() -> App {
    let mut app = App::default();
    app.apply(UiEvent::Upsert {
        meta: InstanceMeta {
            id: "test".to_owned(),
            role: InstanceRole::Portal,
            pid: 42,
            uid: 0,
            version: "test".to_owned(),
            endpoint: "0.0.0.0:2077".to_owned(),
            config_summary: "net=mix tls=1".to_owned(),
            telemetry_interval_ms: 1_000,
        },
        lifecycle: Lifecycle::Ready,
        snapshot: Some(TelemetrySnapshot {
            timestamp_ms: 1_000,
            uptime_ms: 61_000,
            tcp_active: 4,
            udp_active: 2,
            ..TelemetrySnapshot::default()
        }),
    });
    app.apply(UiEvent::Snapshot {
        id: "test".to_owned(),
        snapshot: TelemetrySnapshot {
            timestamp_ms: 2_000,
            uptime_ms: 62_000,
            tcp_rx: 1_000_000,
            tcp_tx: 2_000_000,
            tcp_active: 4,
            udp_active: 2,
            link_tcp: 3,
            link_udp: 1,
            link_pairs: 1,
            pool_active: 4,
            cpu_percent: Some(2.5),
            rss_bytes: Some(42 << 20),
            open_fds: Some(33),
            ..TelemetrySnapshot::default()
        },
    });
    app
}

fn rendered(width: u16, height: u16, app: &App) -> String {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|frame| render(frame, app)).unwrap();
    let buffer = terminal.backend().buffer();
    let mut output = String::new();
    for y in 0..height {
        for x in 0..width {
            output.push_str(buffer[(x, y)].symbol());
        }
        output.push('\n');
    }
    output
}

fn show_logs(app: &mut App) {
    app.page = Page::Logs;
}

#[test]
fn chooses_responsive_breakpoints() {
    assert_eq!(layout_mode(Rect::new(0, 0, 120, 32)), LayoutMode::Full);
    assert_eq!(layout_mode(Rect::new(0, 0, 100, 30)), LayoutMode::Compact);
    assert_eq!(layout_mode(Rect::new(0, 0, 100, 27)), LayoutMode::Narrow);
    assert_eq!(layout_mode(Rect::new(0, 0, 79, 30)), LayoutMode::Narrow);
    assert_eq!(layout_mode(Rect::new(0, 0, 71, 30)), LayoutMode::TooSmall);
    assert_eq!(layout_mode(Rect::new(0, 0, 120, 19)), LayoutMode::TooSmall);
}

#[test]
fn overview_balances_traffic_and_metric_card_heights() {
    assert_eq!(overview_cards_height(30, WorkspaceDensity::Full), 12);
    assert_eq!(overview_cards_height(58, WorkspaceDensity::Full), 18);
    assert_eq!(overview_cards_height(28, WorkspaceDensity::Compact), 11);
    assert_eq!(overview_cards_height(58, WorkspaceDensity::Compact), 15);
}

#[test]
fn renders_full_dashboard() {
    let output = rendered(120, 32, &app_with_instance());
    assert!(output.contains("INSTANCES"));
    assert!(output.contains("Overview"));
    assert!(output.contains("Logs"));
    assert!(output.contains("TRAFFIC"));
    assert!(output.contains("CONNECTIONS"));
    assert!(!output.contains("ACCESS ·"));
    assert!(!output.contains("RUNTIME ·"));
    assert!(output.contains("Portal"));
    assert!(output.contains("DOWN"));
    assert!(output.contains("QUIC"));
    assert!(!output.contains("ACTIVE"));
    assert!(output.contains("CPU"));
    assert!(output.contains("RSS"));
    assert!(output.contains("LST 0.0.0.0:2077"));
    assert!(output.contains("? help"));
    assert!(!output.contains("telemetry 1000ms"));
}

#[test]
fn full_dashboard_uses_equal_cards_and_a_full_height_sidebar() {
    let output = rendered(120, 32, &app_with_instance());
    let cards = output
        .lines()
        .find(|line| {
            line.contains("SELECTED")
                && line.contains("CONNECTIONS")
                && line.contains("CARRIERS / PROCESS")
        })
        .expect("three cards on one row");
    let column = |line: &str, needle: &str| {
        let byte = line.find(needle).expect("title");
        line[..byte].chars().count()
    };
    let selected = column(cards, "SELECTED");
    let connections = column(cards, "CONNECTIONS");
    let carriers = column(cards, "CARRIERS / PROCESS");
    let card_widths = [connections - selected, carriers - connections];
    assert!(card_widths[0].abs_diff(card_widths[1]) <= 1);

    let [instances, workspace] =
        workspace_columns(Rect::new(0, 0, 120, 30), WorkspaceDensity::Full);
    assert_eq!(instances.height, 30);
    assert_eq!(workspace.height, 30);
    assert_eq!(instances.width, 25);
}

#[test]
fn log_page_stacks_full_width_feeds_beside_the_sidebar() {
    let mut app = app_with_instance();
    show_logs(&mut app);
    let output = rendered(120, 32, &app);
    let access_row = output
        .lines()
        .position(|line| line.contains("ACCESS · 0"))
        .expect("access panel");
    let runtime_row = output
        .lines()
        .position(|line| line.contains("RUNTIME · 0"))
        .expect("runtime panel");
    assert!(runtime_row > access_row);
    assert!(
        !output
            .lines()
            .any(|line| { line.contains("ACCESS ·") && line.contains("RUNTIME ·") })
    );

    let [access, runtime] = log_rows(Rect::new(25, 2, 95, 29));
    assert_eq!(access.x, runtime.x);
    assert_eq!(access.width, 95);
    assert_eq!(runtime.width, 95);
    assert!(access.height.abs_diff(runtime.height) <= 1);
}

#[test]
fn compact_layout_keeps_instances_full_height_on_both_pages() {
    let [instances, workspace] =
        workspace_columns(Rect::new(0, 0, 100, 28), WorkspaceDensity::Compact);
    assert_eq!(instances.height, 28);
    assert_eq!(workspace.height, 28);
    assert_eq!(instances.width, 23);

    let mut app = app_with_instance();
    show_logs(&mut app);
    let output = rendered(100, 30, &app);
    assert!(output.contains("INSTANCES"));
    assert!(output.contains("ACCESS · 0"));
    assert!(output.contains("RUNTIME · 0"));
}

#[test]
fn connection_direction_columns_do_not_move_with_rate_width() {
    let mut app = app_with_instance();
    app.instances[0].history.push_back(HistoryPoint {
        upload_bps: 999.0,
        download_bps: 999.0,
        ..HistoryPoint::default()
    });
    let low = rendered(120, 32, &app);
    app.instances[0].history.back_mut().unwrap().upload_bps = 120_000_000.0;
    let high = rendered(120, 32, &app);
    let down_column = |output: &str| {
        output
            .lines()
            .find(|line| line.contains("↑") && line.contains("↓") && line.contains("/s"))
            .and_then(|line| line.find('↓'))
            .expect("connection rate row")
    };
    assert_eq!(down_column(&low), down_column(&high));
}

#[test]
fn renders_narrow_two_page_dashboard() {
    let output = rendered(72, 20, &app_with_instance());
    assert!(output.contains("Overview"));
    assert!(output.contains("SELECTED"));
    assert!(output.contains("0.0.0.0:2077"));
    assert!(output.contains("1000ms"));
    assert!(output.contains("? help"));
    assert!(!output.contains("telemetry 1000ms"));
}

#[test]
fn tiny_terminal_shows_resize_message() {
    let output = rendered(50, 14, &app_with_instance());
    assert!(output.contains("Minimum is 72"));
}

#[test]
fn access_shows_only_source_and_target() {
    let mut app = app_with_instance();
    show_logs(&mut app);
    app.apply(UiEvent::Access {
        id: "test".to_owned(),
        record: AccessRecord {
            timestamp_ms: 1,
            event_id: 7,
            phase: AccessPhase::Start,
            protocol: "TCP".to_owned(),
            client: Some("10.20.30.40:1234".to_owned()),
            path_peers: vec!["10.20.30.40:1234".to_owned(), "10.20.30.40:5678".to_owned()],
            route: "UP 10.20.30.40:1234 -> relay | DOWN relay -> 10.20.30.40:5678".to_owned(),
            target: Some("example:443".to_owned()),
            ..AccessRecord::default()
        },
    });
    let masked = rendered(200, 32, &app);
    assert!(masked.contains("10.20.x.x:1234"));
    assert!(masked.contains("example:443"));
    assert!(!masked.contains("10.20.30.40:1234"));
    assert!(!masked.contains("10.20.30.40:5678"));
    assert!(!masked.contains("relay"));

    app.reveal_clients = true;
    let revealed = rendered(200, 32, &app);
    assert!(revealed.contains("10.20.30.40:1234"));
    assert!(!revealed.contains("10.20.30.40:5678"));
    assert!(!revealed.contains("relay"));
}

#[test]
fn selected_config_wraps_without_internal_history_counters() {
    let mut app = app_with_instance();
    app.instances[0].meta.config_summary =
        "mode=reverse transport=quic tls=enabled pool=16".to_owned();
    let output = rendered(120, 32, &app);
    assert!(output.contains("mode=reverse"));
    assert!(output.contains("pool=16"));
    assert!(!output.contains("HIST"));
    assert!(!output.contains("FEED A/R"));
    assert!(!output.contains("GAP/OVR"));
}

#[test]
fn carrier_process_metrics_use_three_balanced_graph_rows() {
    let output = rendered(120, 32, &app_with_instance());
    let carrier_row = output
        .lines()
        .find(|line| line.contains(" TLS") && line.contains(" QUIC"))
        .expect("TLS and QUIC row");
    assert!(carrier_row.find(" TLS") < carrier_row.find(" QUIC"));
    let process_row = output
        .lines()
        .find(|line| line.contains(" PAIR") && line.contains(" POOL"))
        .expect("PAIR and POOL row");
    assert!(process_row.find(" PAIR") < process_row.find(" POOL"));
    let resource_row = output
        .lines()
        .find(|line| line.contains(" CPU") && line.contains(" RSS"))
        .expect("CPU and RSS row");
    assert!(resource_row.find(" CPU") < resource_row.find(" RSS"));
    assert!(!output.contains(" FD"));
    assert!(!output.contains(" SEQ"));
}

#[test]
fn carrier_and_process_second_columns_share_one_alignment() {
    let output = rendered(120, 32, &app_with_instance());
    let column = |line: &str, needle: &str| {
        let byte = line.find(needle).expect("metric");
        line[..byte].chars().count()
    };
    let quic_row = output
        .lines()
        .find(|line| line.contains(" TLS") && line.contains(" QUIC"))
        .expect("TLS/QUIC row");
    let pool_row = output
        .lines()
        .find(|line| line.contains(" PAIR") && line.contains(" POOL"))
        .expect("PAIR/POOL row");
    let rss_row = output
        .lines()
        .find(|line| line.contains(" CPU") && line.contains(" RSS"))
        .expect("CPU/RSS row");
    assert_eq!(column(quic_row, " QUIC"), column(pool_row, " POOL"));
    assert_eq!(column(pool_row, " POOL"), column(rss_row, " RSS"));
}

#[test]
fn runtime_peer_is_masked_until_revealed() {
    let mut app = app_with_instance();
    app.apply(UiEvent::Runtime {
        id: "test".to_owned(),
        record: RuntimeRecord {
            timestamp_ms: 1,
            level: EventLevel::Warn,
            kind: "AUTHENTICATION".to_owned(),
            message: "handshake failed".to_owned(),
            client: Some("10.20.30.40:1234".to_owned()),
        },
    });
    app.set_feed(FeedKind::Runtime);
    let masked = rendered(120, 32, &app);
    assert!(masked.contains("10.20.x.x:1234"));
    assert!(!masked.contains("10.20.30.40:1234"));

    app.reveal_clients = true;
    assert!(rendered(120, 32, &app).contains("10.20.30.40:1234"));
}

#[test]
fn runtime_tail_is_available_through_horizontal_scrolling() {
    let mut app = app_with_instance();
    app.apply(UiEvent::Runtime {
        id: "test".to_owned(),
        record: RuntimeRecord {
            timestamp_ms: 1,
            level: EventLevel::Info,
            kind: "CARRIER".to_owned(),
            message:
                "carrier handshake exceeded the usual row width; accepted on second-row-marker"
                    .to_owned(),
            client: None,
        },
    });
    app.set_feed(FeedKind::Runtime);

    assert!(!rendered(72, 20, &app).contains("second-row-marker"));
    app.runtime_horizontal_scroll = 60;
    assert!(rendered(72, 20, &app).contains("second-row-marker"));
}

#[test]
fn access_prioritizes_complete_route_over_optional_stats() {
    let mut app = app_with_instance();
    show_logs(&mut app);
    app.apply(UiEvent::Access {
        id: "test".to_owned(),
        record: AccessRecord {
            timestamp_ms: 1,
            event_id: 8,
            phase: AccessPhase::Finish,
            protocol: "TCP".to_owned(),
            client: Some("10.20.30.40:1234".to_owned()),
            target: Some("destination.example:443".to_owned()),
            status: Some(AccessStatus::Success),
            duration_ms: Some(42_000),
            upload_bytes: Some(1 << 30),
            download_bytes: Some(2 << 30),
            ..AccessRecord::default()
        },
    });

    let output = rendered(200, 32, &app);
    assert!(output.contains("10.20.x.x:1234"));
    assert!(output.contains("destination.example:443"));
}

#[test]
fn access_error_message_stays_on_the_status_row() {
    let mut app = app_with_instance();
    show_logs(&mut app);
    app.apply(UiEvent::Access {
        id: "test".to_owned(),
        record: AccessRecord {
            timestamp_ms: 1,
            event_id: 9,
            phase: AccessPhase::Finish,
            protocol: "TCP".to_owned(),
            client: Some("10.20.30.40:1234".to_owned()),
            target: Some("example:443".to_owned()),
            status: Some(AccessStatus::Error),
            message: Some("dial failed: dedicated-error-row".to_owned()),
            ..AccessRecord::default()
        },
    });

    let output = rendered(240, 32, &app);
    assert!(
        output
            .lines()
            .any(|line| line.contains("ERR") && line.contains("dedicated-error-row"))
    );
}

#[test]
fn runtime_error_message_stays_on_the_event_row() {
    let mut app = app_with_instance();
    show_logs(&mut app);
    app.apply(UiEvent::Runtime {
        id: "test".to_owned(),
        record: RuntimeRecord {
            timestamp_ms: 1,
            level: EventLevel::Error,
            kind: "IPC".to_owned(),
            message: "connection failed: dedicated-runtime-row".to_owned(),
            client: None,
        },
    });

    let output = rendered(240, 32, &app);
    assert!(
        output
            .lines()
            .any(|line| line.contains("IPC") && line.contains("dedicated-runtime-row"))
    );
}

#[test]
fn benign_access_completion_is_a_quiet_end() {
    let mut app = app_with_instance();
    show_logs(&mut app);
    app.apply(UiEvent::Access {
        id: "test".to_owned(),
        record: AccessRecord {
            timestamp_ms: 1,
            event_id: 8,
            phase: AccessPhase::Finish,
            protocol: "TCP".to_owned(),
            client: Some("10.20.30.40:1234".to_owned()),
            target: Some("example:443".to_owned()),
            status: Some(AccessStatus::Ended),
            message: None,
            ..AccessRecord::default()
        },
    });

    let output = rendered(120, 32, &app);
    assert!(output.contains("END"));
    assert!(!output.contains("error 256"));
}

#[test]
fn downsampling_keeps_peaks_and_width_bound() {
    let history = (0..100)
        .map(|index| HistoryPoint {
            timestamp_ms: index,
            upload_bps: if index == 55 { 1_000.0 } else { index as f64 },
            download_bps: index as f64,
            ..HistoryPoint::default()
        })
        .collect::<VecDeque<_>>();
    let series = rate_series(&history, 10, |point| point.upload_bps);
    assert_eq!(series.len(), 10);
    assert!(series.contains(&1_000));
}

#[test]
fn all_six_traffic_series_have_distinct_colors() {
    for (index, color) in TRAFFIC_COLORS.iter().enumerate() {
        assert!(
            TRAFFIC_COLORS[index + 1..]
                .iter()
                .all(|candidate| candidate != color)
        );
    }
}

#[test]
fn graph_styles_match_each_metrics_visual_role() {
    assert_eq!(TRAFFIC_GRAPH_STYLE, GraphStyle::Filled);
    assert_eq!(CONNECTION_GRAPH_STYLE, GraphStyle::Hollow);
    assert_eq!(PROCESS_GRAPH_STYLE, GraphStyle::Hollow);
}

#[test]
fn traffic_grid_has_a_cell_between_every_chart_column() {
    let cells = traffic_cells(Rect::new(0, 0, 90, 12), true);
    assert_eq!(cells[1].x, cells[0].right() + 1);
    assert_eq!(cells[2].x, cells[1].right() + 1);
    assert_eq!(cells[4].x, cells[3].right() + 1);
    assert_eq!(cells[5].x, cells[4].right() + 1);
}
