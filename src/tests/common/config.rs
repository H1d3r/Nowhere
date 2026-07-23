// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

//! Configuration parsing tests.

use super::*;
use url::Url;

#[test]
fn query_first_ignores_unknown_parameters_and_keeps_first_duplicate() {
    let parsed = Url::parse("portal://key@localhost:2077?log=debug&alpn=now%2F1").unwrap();
    let values = query_first(&parsed, &["log", "alpn"]).unwrap();
    assert_eq!(values["log"], "debug");
    assert_eq!(values["alpn"], "now/1");

    let duplicate = Url::parse("portal://key@localhost:2077?log=debug&log=event").unwrap();
    assert_eq!(query_first(&duplicate, &["log"]).unwrap()["log"], "debug");
    let unknown = Url::parse("portal://key@localhost:2077?typo=value&%FF=value").unwrap();
    assert!(query_first(&unknown, &["log"]).unwrap().is_empty());
}

#[test]
fn query_first_preserves_literal_plus_and_validates_the_selected_value() {
    let parsed = Url::parse("portal://key@localhost:2077?alpn=now+private").unwrap();
    assert_eq!(
        query_first(&parsed, &["alpn"]).unwrap()["alpn"],
        "now+private"
    );

    let bad = Url::parse("portal://key@localhost:2077?alpn=%GG").unwrap();
    assert!(query_first(&bad, &["alpn"]).is_err());

    let ignored_bad_duplicate =
        Url::parse("portal://key@localhost:2077?alpn=now%2F1&alpn=%GG").unwrap();
    assert_eq!(
        query_first(&ignored_bad_duplicate, &["alpn"]).unwrap()["alpn"],
        "now/1"
    );
}

#[test]
fn init_dialer_ip_accepts_only_ip_literals() {
    assert_eq!(init_dialer_ip(Some("127.0.0.1")), "127.0.0.1");
    assert_eq!(init_dialer_ip(Some("::1")), "::1");
    assert_eq!(init_dialer_ip(Some(DEFAULT_DIALER_IP)), DEFAULT_DIALER_IP);
    assert_eq!(init_dialer_ip(Some("example.com")), DEFAULT_DIALER_IP);
    assert_eq!(init_dialer_ip(None), DEFAULT_DIALER_IP);
}

#[test]
fn rate_limit_converts_mbps_to_bytes_per_second() {
    assert_eq!(rate_limit_bytes_per_second(-1), 0);
    assert_eq!(rate_limit_bytes_per_second(0), 0);
    assert_eq!(rate_limit_bytes_per_second(1), 125_000);
    assert_eq!(rate_limit_bytes_per_second(8), 1_000_000);
}

#[test]
fn flow_setup_timeout_defaults_and_accepts_override() {
    unsafe { std::env::remove_var("NOW_FLOW_SETUP_TIMEOUT") };
    assert_eq!(flow_setup_timeout(), Duration::from_secs(20));

    unsafe { std::env::set_var("NOW_FLOW_SETUP_TIMEOUT", "750ms") };
    assert_eq!(flow_setup_timeout(), Duration::from_millis(750));
    unsafe { std::env::remove_var("NOW_FLOW_SETUP_TIMEOUT") };
}
