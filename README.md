# Nowhere

> **One port. Two transports. Split directions.**

Nowhere brings TLS/TCP and QUIC/UDP together on one service port for both TCP
and UDP traffic. Each logical flow composes its upload and download paths
independently, allowing the two directions to use different carriers.

<div align="center">
  <img src="assets/nowhere.png" width="640" alt="Nowhere">
</div>

## What Nowhere Does

- **One service port.** TLS/TCP and QUIC/UDP share the same listener address,
  service port, credentials, and operational lifecycle.
- **Independent directions.** Upload and download can use different carriers,
  so transport selection follows the needs of each direction rather than the
  whole connection.
- **TCP and UDP coverage.** TCP uses TLS connections or QUIC streams. UDP uses
  UoT over TLS/TCP or QUIC DATAGRAM.
- **Lean wire frames.** A 32-byte connection-auth frame leads into a 5-byte
  flow header and one-byte setup result. Common QUIC DATAGRAM and UoT packets
  add only 5 and 2 bytes, respectively.
- **Efficient hot path.** Stack-encoded headers, binary targets, allocation-free
  DATAGRAM decoding, reusable buffers, shared QUIC connections, and prepared
  TLS lanes reduce parsing, copying, allocation, and connection setup work.
- **Production controls.** Directional rate limits, warm TLS lanes, outbound
  SOCKS5, source binding, certificate reload, resource limits, access paths,
  EVENT telemetry, and graceful shutdown are built in.

## Directional Transport

The Vector `up` and `down` parameters select carriers independently:

| Vector mode | Upload | Download |
| --- | --- | --- |
| `tcp/tcp` | TLS/TCP | TLS/TCP |
| `tcp/udp` | TLS/TCP | QUIC/UDP |
| `udp/tcp` | QUIC/UDP | TLS/TCP |
| `udp/udp` | QUIC/UDP | QUIC/UDP |

For TCP traffic, the QUIC carrier is a bidirectional stream. For UDP traffic,
the TLS/TCP carrier uses length-prefixed UoT and the QUIC carrier uses
DATAGRAM. Split flows are joined by their authenticated session and flow
identity, not by source address.

Portal `net=mix`, the default, accepts both carrier families on the same port.
`net=tcp` and `net=udp` are available when an operator intentionally wants only
one listener transport.

## Data Path

Authentication is bound to each TLS or QUIC connection through a TLS exporter.
A shared QUIC connection can carry many streams and UDP flows, while the
`tcp/tcp` warm pool can prepare authenticated TLS lanes before an application
requests them. Binary target addressing and compact setup metadata keep relay
work direct, and explicit limits keep connection, flow, queue, and reassembly
state bounded.

## Components

- `portal://` accepts encrypted carriers and dials target endpoints.
- `vector://` connects to Portal and serves a local SOCKS5 endpoint.
- [Anywhere](https://github.com/NodePassProject/Anywhere) is the Apple client.
  See the [integration guide](docs/integrations.md) for release compatibility.

## Quick Start

Build with a stable Rust toolchain:

```bash
cargo build --release --locked
```

Start a local Portal:

```bash
./target/release/nowhere 'portal://change-me@127.0.0.1:2077'
```

Start Vector with TLS/TCP in both directions and five prepared lanes:

```bash
./target/release/nowhere \
  'vector://change-me@127.0.0.1:2077?up=tcp&down=tcp&pool=5&socks=127.0.0.1:1080'
```

Or use QUIC/UDP for upload and TLS/TCP for download:

```bash
./target/release/nowhere \
  'vector://change-me@127.0.0.1:2077?up=udp&down=tcp&socks=127.0.0.1:1080'
```

The local examples omit `sni`, which disables certificate verification. For a
public Portal, install a CA-trusted certificate and enable strict verification:

```bash
nowhere 'portal://change-me@:2077?tls=2&crt=/etc/nowhere/cert.pem&key=/etc/nowhere/key.pem'
nowhere 'vector://change-me@relay.example:2077?sni=relay.example&socks=127.0.0.1:1080'
```

Portal and Vector default to ALPN `now/1`. A custom `alpn` must match on both
ends.

## Documentation

- [Documentation index](docs/README.md)
- [Quick start](docs/quick-start.md)
- [Configuration reference](docs/configuration.md)
- [Operations guide](docs/operations.md)
- [Security model](docs/security.md)
- [Protocol specification](docs/protocol.md)
- [Integration guide](docs/integrations.md)

## Development

```bash
cargo fmt --all -- --check
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
cargo build --release --locked
```

Protocol changes must update the normative document and wire-vector tests in
the same change.

## License

Nowhere is licensed under the [GNU General Public License v3.0](LICENSE).
Distributions of original or modified binaries must comply with the GPLv3
source and notice requirements.

---

© 2026 NodePassProject. All rights reserved.
