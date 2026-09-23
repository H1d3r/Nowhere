use std::collections::VecDeque;

use ratatui::Terminal;
use ratatui::backend::TestBackend;

use super::graphs::{
    CONNECTION_GRAPH_STYLE, GraphStyle, PROCESS_GRAPH_STYLE, TRAFFIC_COLORS, TRAFFIC_GRAPH_STYLE,
    rate_series, traffic_cells,
};
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
            endpoint: "0.0.0.0:2000".to_owned(),
            config_summary: "net=mix tls=1".to_owned(),
            telemetry_interval_ms: 1_000,
            telemetry_protocol: "nowhere.telemetry".to_owned(),
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
            tcp_logical_up: 1_000_000,
            tcp_logical_down: 2_000_000,
            tcp_active: 4,
            udp_active: 2,
            tls_carriers_active: 3,
            quic_carriers_active: 1,
            cpu_percent: Some(2.5),
            rss_bytes: Some(42 << 20),
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

#[path = "render/completion_and_graphs.rs"]
mod completion_and_graphs;
#[path = "render/config.rs"]
mod config;
#[path = "render/dashboard.rs"]
mod dashboard;
#[path = "render/feed.rs"]
mod feed;
