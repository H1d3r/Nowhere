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
fn masks_ipv4_and_ipv6_clients() {
    assert_eq!(client_address("10.20.30.40:1234", false), "10.20.x.x:1234");
    assert_eq!(
        client_address("[2001:db8:1234:5678::1]:443", false),
        "[2001:db8:1234:…]:443"
    );
    assert_eq!(client_address("10.20.30.40:1234", true), "10.20.30.40:1234");
}

#[test]
fn truncates_on_character_boundaries() {
    assert_eq!(truncate("Nowhere", 5), "Nowh…");
    assert_eq!(truncate("遥测数据", 3), "遥测…");
}
