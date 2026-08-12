<p align="center">
  <img src="assets/nowhere.png" width="540" alt="Nowhere">
</p>

<p align="center">
  <strong>One port. Two transports. Split directions.</strong>
</p>

<p align="center">
  A Linux-native encrypted relay that composes TLS/TCP and QUIC/UDP<br>
  independently for upload and download.
</p>

<p align="center">
  <a href="#live-operations">Live operations</a> &middot;
  <a href="#how-it-works">Architecture</a> &middot;
  <a href="#quick-start">Quick start</a> &middot;
  <a href="docs/README.md">Documentation</a> &middot;
  <a href="docs/protocol.md">Wire protocol</a>
</p>

Nowhere gives one service edge two encrypted carrier families. A local
**Vector** accepts SOCKS5 traffic; a remote **Portal** authenticates carriers,
opens targets, and relays data. Every logical flow chooses its uplink and
downlink independently instead of forcing both directions onto one transport.

| Core property | What it means |
| --- | --- |
| One service edge | TLS/TCP and QUIC/UDP share one address, port number, credential, and lifecycle |
| Split directions | Uplink and downlink independently select TLS/TCP or QUIC/UDP |
| Complete ingress | SOCKS5 CONNECT carries TCP; UDP ASSOCIATE carries UDP |
| Native chaining | A Portal can forward directly to another Portal without a loopback SOCKS5 conversion |
| Local observability | The same binary discovers running instances and renders live telemetry metrics |

## Live operations

<p align="center">
  <img src="assets/nowhere.gif" width="1280" alt="Nowhere TUI showing six live traffic histories, native Portal chaining, upstream RTT, connection and carrier metrics, privacy-aware access logs, runtime events, filtering, pause, and help">
</p>

The built-in TUI turns every visible Portal and Vector into a live operational
view. It remains deliberately separate from process management: opening or
closing a dashboard never starts, stops, or owns a service instance.

| View | Signals and controls |
| --- | --- |
| **Overview** | Uplink, downlink, TCP, UDP, TLS, and QUIC histories; active connections; carriers; upstream RTT; warm pool; CPU; RSS; selected-instance metadata |
| **Logs** | Independent Access and Runtime feeds with filtering, pause and resume, paging, horizontal panning, and local privacy masking |
| **Discovery** | Automatic instance discovery with stable selection and concurrent read-only viewers |

Start it from any terminal in the same Linux environment:

```bash
nowhere
# Equivalent explicit form
nowhere tui
```

Telemetry uses a local abstract Unix socket. It does not consume or redirect
stdout and stderr, and history is held only by connected dashboards.

## How it works

```text
  Application
   TCP / UDP
       |
     SOCKS5
       |
       v
+--------------+   Uplink: TLS/TCP or QUIC/UDP    +--------------+
|              |=================================>|              |
|    Vector    |                                  |    Portal    |
|              |<=================================|              |
+--------------+  Downlink: TLS/TCP or QUIC/UDP   +--------------+
                                                          |
                                                  direct or SOCKS5
                                                          |
                                                          v
                                                    +------------+
                                                    |   Target   |
                                                    +------------+
```

Portal defaults to `net=mix`, accepting both carrier families on the same port
number. `net=tcp` and `net=udp` intentionally restrict the listener when an
operator wants only one carrier family.

### One flow, two transport decisions

Vector's `up` and `down` parameters form four first-class modes:

| Mode | Uplink | Downlink |
| --- | --- | --- |
| `tcp/tcp` | TLS/TCP | TLS/TCP |
| `tcp/udp` | TLS/TCP | QUIC/UDP |
| `udp/tcp` | QUIC/UDP | TLS/TCP |
| `udp/udp` | QUIC/UDP | QUIC/UDP |

TCP application traffic uses a TLS connection or a bidirectional QUIC stream.
UDP application traffic uses length-prefixed UoT over TLS/TCP or QUIC
DATAGRAM. Split paths rejoin through authenticated session and flow identity,
never by source address.

## Engineered for a small data path

| Area | Design |
| --- | --- |
| Framing | 32-byte connection authentication, 5-byte flow header, and one-byte setup result |
| UDP overhead | 5 bytes for common QUIC DATAGRAM packets and 2 bytes for UoT packets |
| Hot path | Stack-encoded headers, binary targets, allocation-free DATAGRAM decoding, and reusable buffers |
| Reuse | Shared QUIC connections carry many streams and UDP flows; warm TLS lanes reduce `tcp/tcp` setup work |
| Authentication | Credentials are bound to each TLS or QUIC connection through a TLS exporter |
| Resource control | Explicit bounds cover connections, flows, queues, reassembly, rate limits, and idle state |

Certificate reload, graceful shutdown, outbound SOCKS5, source binding,
directional rate limits, access paths, and EVENT logging are part of the core
runtime rather than external wrappers.

### Native Portal chaining

A relay Portal can terminate the incoming TLS/QUIC carrier and open the next
Nowhere flow directly with the same transport engine used by Vector:

```bash
nowhere \
  'portal://relay-key@:2077?next=origin-key@origin.example:2077&up=udp&down=udp'
```

`next` is mutually exclusive with outbound `socks`. It is lazy, so an
unavailable upstream never prevents the relay listener from becoming ready.
TCP and UDP payloads remain in the native binary flow path—there is no local
SOCKS listener, SOCKS framing, or per-packet connection setup between Portals.
Native forwarding is bounded to seven Portal hops.

## Quick start

Nowhere requires Linux and a stable Rust toolchain.

### 1. Build

```bash
cargo build --release --locked
```

### 2. Start Portal

The default `net=mix` mode accepts TLS/TCP and QUIC/UDP on port `2077`:

```bash
./target/release/nowhere 'portal://change-me@127.0.0.1:2077'
```

### 3. Start Vector

This Vector exposes SOCKS5 on `127.0.0.1:1080`, uses TLS/TCP in both
directions, and keeps five authenticated TLS lanes warm:

```bash
./target/release/nowhere \
  'vector://change-me@127.0.0.1:2077?up=tcp&down=tcp&pool=5&socks=127.0.0.1:1080'
```

To split the carriers, change the directional parameters:

```bash
./target/release/nowhere \
  'vector://change-me@127.0.0.1:2077?up=udp&down=tcp&socks=127.0.0.1:1080'
```

### 4. Inspect

Open another terminal and run:

```bash
./target/release/nowhere tui
```

## Before public deployment

The local examples omit `sni`, which disables certificate verification. A
public Portal should use a CA-trusted certificate with strict verification:

```bash
nowhere 'portal://change-me@:2077?tls=2&crt=/etc/nowhere/cert.pem&key=/etc/nowhere/key.pem'
nowhere 'vector://change-me@relay.example:2077?sni=relay.example&socks=127.0.0.1:1080'
```

Alternatively, set `pin` to the lowercase `CERT_SHA256` value printed by
Portal. Pinning takes priority over `sni` certificate-chain and name checks.
Portal and Vector default to ALPN `now/1`; custom ALPN values must match at both
ends.

Read the [security model](docs/security.md) before exposing a Portal outside a
trusted environment.

## Operational boundaries

| Boundary | Behavior |
| --- | --- |
| Platform | Linux only; runtime, signals, process metrics, discovery, and telemetry use Linux APIs directly |
| Visibility | An unprivileged TUI sees instances owned by its effective UID; root sees all instances visible in its PID and network namespaces |
| Ownership | The TUI is read-only and has no service lifecycle or configuration authority |
| Concurrency | Multiple TUIs may observe one instance at the same time |
| Logging | stdout and stderr remain independent from structured TUI telemetry |
| Persistence | TUI metric history and feeds begin at connection time and are not persisted |

Containers provide the portability boundary for non-Linux hosts while keeping
the Linux process and IPC model intact inside each environment.

## Documentation map

| Guide | Start here when you need to |
| --- | --- |
| [Quick start](docs/quick-start.md) | Build a local Portal and Vector |
| [Configuration](docs/configuration.md) | Review command URLs, defaults, identity, limits, and environment variables |
| [Operations](docs/operations.md) | Operate logs, telemetry, pools, reconnection, certificates, and shutdown |
| [Security](docs/security.md) | Understand trust boundaries, authentication, permissions, and resource controls |
| [Wire protocol](docs/protocol.md) | Implement authentication, flow setup, framing, and lifecycles |
| [Integrations](docs/integrations.md) | Connect process managers, SOCKS5 clients, OpenCtrl, and other tooling |

## Development

On Linux:

```bash
cargo fmt --all -- --check
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
cargo build --release --locked
```

On macOS with [Apple Container](https://github.com/apple/container), run the
same Linux checks through the reusable, cache-backed environment:

```bash
./scripts/check-linux.sh
```

The script pins Rust 1.95.0 by default, mounts the source read-only, and keeps
Cargo, rustup, and target caches in named volumes. Override its resources or
toolchain with `NOWHERE_CONTAINER_CPUS`, `NOWHERE_CONTAINER_MEMORY`,
`NOWHERE_LINUX_IMAGE`, and `NOWHERE_RUST_TOOLCHAIN`.

Protocol changes must update the normative wire document and protocol-vector
tests in the same change.

## License

Nowhere is licensed under the [GNU General Public License v3.0](LICENSE).
Distributions of original or modified binaries must comply with the GPLv3
source and notice requirements.

---

© 2026 NodePassProject. All rights reserved.
