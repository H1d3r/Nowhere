// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

use super::*;

#[test]
fn config_overlay_keeps_complete_values_and_scrolls_to_last_option() {
    let mut app = app_with_instance();
    app.instances[0].meta.config_summary = "portal=a-very-long-relay-hostname.example:2077 up=udp down=tcp mux=1 morph=1 socks=127.0.0.1:1080 rate=100 etar=200 sni=relay.example pin=present".into();
    app.show_config = true;
    let output = rendered(72, 20, &app);
    assert!(output.contains("a-very-long-relay-hostname.example:2077"));
    assert!(output.contains("0.0.0.0:2000"));
    app.config_scroll = 10;
    let output = rendered(72, 20, &app);
    assert!(output.contains("pin"));
    assert!(output.contains("present"));
}

#[test]
fn selected_config_wraps_long_values_instead_of_dropping_their_tail() {
    let lines = super::metrics::wrap_tokens("portal=long-hostname.example:2077 mux=1", 16, 4);
    assert_eq!(
        lines.join("").replace(' ', ""),
        "portal=long-hostname.example:2077mux=1"
    );
}

#[test]
fn selected_config_wraps_without_internal_history_counters() {
    let mut app = app_with_instance();
    app.instances[0].meta.config_summary = "mode=reverse transport=quic tls=enabled".to_owned();
    let output = rendered(120, 32, &app);
    assert!(output.contains("mode=reverse"));
    assert!(!output.contains("HIST"));
    assert!(!output.contains("FEED A/R"));
    assert!(!output.contains("GAP/OVR"));
}

#[test]
fn selected_uses_available_height_for_complete_config() {
    let mut app = app_with_instance();
    app.instances[0].meta.config_summary =
        "net=mix tls=1 rate=0 etar=0 dial=auto socks=none next=origin.example:3077 up=udp down=tcp mux=0 sni=origin.example pin=present"
            .to_owned();

    let output = rendered(160, 40, &app);
    assert!(output.contains("sni=origin.example"));
    assert!(output.contains("pin=present"));
}
