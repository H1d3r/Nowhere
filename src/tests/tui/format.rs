use super::*;

#[test]
fn formats_rates_with_si_units() {
    assert_eq!(bits_per_second(0.0), "0 b/s");
    assert_eq!(bits_per_second(999.0), "999 b/s");
    assert_eq!(bits_per_second(1_500_000.0), "1.50 Mb/s");
    assert_eq!(bits_per_second(120_000_000.0), "120 Mb/s");
}

#[test]
fn formats_byte_totals_with_iec_units() {
    assert_eq!(bytes(0), "0 B");
    assert_eq!(bytes(1_536), "1.50 KiB");
    assert_eq!(bytes(10 * (1 << 20)), "10.0 MiB");
}

#[test]
fn renders_only_anonymous_clients() {
    assert_eq!(client_address("10.20.30.40:1234"), "<redacted>");
    assert_eq!(client_address("[2001:db8::1]:443"), "<redacted>");
    assert_eq!(client_address("C001"), "C001");
    assert_eq!(client_address("C1000"), "C1000");
    for invalid in [
        "C000",
        "C",
        "Csecret",
        "C001\x1b[31m",
        "client_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    ] {
        assert_eq!(client_address(invalid), "<redacted>");
    }
}

#[test]
fn truncates_on_character_boundaries() {
    assert_eq!(truncate("Nowhere", 5), "Nowh…");
    assert_eq!(truncate("遥测数据", 3), "遥测…");
}

#[test]
fn abbreviated_instance_addresses_keep_single_and_dual_ports() {
    assert_eq!(instance_endpoint("0.0.0.0:2077", 20), "0.0.0.0:2077");
    let single = instance_endpoint("a-very-long-relay.example:2077", 20);
    assert!(single.ends_with(":2077"));
    assert_eq!(single.chars().count(), 20);
    let dual = instance_endpoint("a-very-long-relay.example/tcp4:2077/udp6:3077", 20);
    assert!(dual.ends_with(":2077/3077"));
    assert_eq!(dual.chars().count(), 20);
}

#[test]
fn abbreviated_ipv6_addresses_keep_brackets_and_port() {
    let address = instance_endpoint("[2001:db8:abcd:1234::1]:2077", 20);
    assert!(address.starts_with('['));
    assert!(address.ends_with("]:2077"));
    assert_eq!(address.chars().count(), 20);
    for width in 0..8 {
        assert!(
            instance_endpoint("relay.example:2077", width)
                .chars()
                .count()
                <= width
        );
    }
}
