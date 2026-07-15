# Integrations

Nowhere exposes two process roles and one wire contract. Process managers own
the command URL and lifecycle; applications reach Vector through SOCKS5;
protocol clients connect directly to Portal.

## Process Management

A service manager should store the complete command URL outside the repository,
restrict access to it, validate it before restart, and preserve stdout for
operational records.

Portal example:

```text
portal://change-me@:2077?net=mix&tls=2&crt=/etc/nowhere/fullchain.pem&key=/etc/nowhere/privkey.pem&alpn=now%2F1
```

Vector example:

```text
vector://change-me@relay.example:2077?up=tcp&down=tcp&pool=5&sni=relay.example&alpn=now%2F1&rate=0&etar=0&socks=127.0.0.1:1080&log=event
```

The URL username is the shared key. Do not expose it through world-readable
unit files, process dashboards, crash reports, or command inventories.

## OpenCtrl

[OpenCtrl](https://github.com/NodePassProject/OpenCtrl) can supervise Portal
processes, persist command URLs, collect stdout, and consume EVENT records over
its management interfaces. OpenCtrl does not terminate Nowhere carriers or
change protocol semantics; the managed process remains the source of transport
state and flow telemetry.

A controller should verify:

1. command URL validation succeeds;
2. the process emits its credential-free effective URL;
3. CHECK_POINT records continue at the configured interval;
4. both TCP and UDP application flows complete through the selected carriers.

## Vector SOCKS5

Vector is the application-facing integration point:

- CONNECT opens one Nowhere TCP flow.
- UDP ASSOCIATE creates idle-timed Nowhere UDP flows per target address.
- One association may communicate with multiple targets.
- RFC1929 is enabled by percent-encoded credentials in `socks`.
- Configured credentials cannot downgrade to no-auth.
- BIND and SOCKS5 UDP fragmentation are rejected.

Use a loopback listener by default. A wildcard listener should be protected by
RFC1929 credentials and host firewall rules.

## Client Implementation Contract

A direct client MUST implement the complete wire protocol described in
[Wire Protocol](protocol.md), including:

- TLS 1.3 and matching ALPN;
- TLS-exporter-bound authentication on every physical connection;
- the 5-byte flow header and its role/carrier validation;
- binary IPv4, IPv6, and domain targets;
- the 1-byte setup result;
- 5-byte DATA/CLOSE and 13-byte FRAGMENT QUIC DATAGRAM headers;
- READY gating before QUIC UDP DATA;
- length-prefixed UoT packets;
- bounded flow, queue, and reassembly state.

ALPN selects the application protocol during TLS or QUIC negotiation. A client
is ready to interoperate only when its frame encoders, decoders, lifecycle, and
error handling satisfy the same wire contract.
