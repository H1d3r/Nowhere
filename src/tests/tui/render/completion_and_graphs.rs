use super::*;

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
