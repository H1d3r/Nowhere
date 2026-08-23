# Configuration

URLs and environment variables have the same meaning on Linux, macOS, and Windows. Shell quoting and filesystem path syntax follow the host platform; see [Platforms](platforms.md).

## Portal URL

```text
portal://shared-key@host:port?net=mix&tls=1&log=info
```

| Query | Values | Default |
|---|---|---|
| `net` | `mix`, `tcp`, `udp` | `mix` |
| `tls` | `1` generated certificate, `2` supplied certificate | `1` |
| `crt`, `key` | PEM paths, required with `tls=2` | — |
| `alpn` | exact TLS/QUIC ALPN, 1–255 bytes | `now/1` |
| `rate`, `etar` | Mbps, `0` disables limit | `0` |
| `dial` | `auto` or local IP | `auto` |
| `socks` | outbound SOCKS5 configuration | disabled |
| `next` | `shared-key@host:port` | disabled |
| `mux` | native next-hop TLS: `0` dedicated lanes, `1` Mux | `0` |
| `log` | `none`, `debug`, `info`, `warn`, `error`, `event` | `info` |

When `next` is enabled, `up`, `down`, `mux`, `sni`, and `pin` configure that
upstream hop. The Portal's `alpn` also applies to its native upstream client.
These upstream options are ignored when `next` is absent or `none`.

## Vector URL

```text
vector://shared-key@host:port?up=tcp&down=tcp&socks=127.0.0.1:1080
```

| Query | Values | Default |
|---|---|---|
| `up`, `down` | `tcp` or `udp` | `udp` |
| `alpn` | exact TLS/QUIC ALPN, 1–255 bytes | `now/1` |
| `mux` | `0` dedicated TLS lanes, `1` TLS Mux | `0` |
| `sni` | verified DNS name, or `none` | `none` |
| `pin` | certificate SHA-256 pin, or `none` | `none` |
| `rate`, `etar` | Mbps, `0` disables limit | `0` |
| `socks` | required local listen address, optionally credentials | — |
| `log` | logging threshold | `info` |

With `mux=1`, Shards open lazily according to active flow pressure. New flows
use the least-loaded shard; a shard carries 12 active flows before another
opens and closes after 30 seconds fully idle. With `mux=0`, every TLS-carried
Flow owns one on-demand lane that closes with the Flow.

Portal and Vector advertise only their configured ALPN and require an exact
match. ALPN and Mux are independent settings. Portal's `mux` option controls
only its `next` client. Inbound Portal connections accept a `0xff`-marked Mux
carrier or an unmarked dedicated lane on the same listener.

For `tls=2`, `crt` and `key` are native filesystem paths. Quote the complete URL when a Windows path, space, `&`, or another shell-significant character is present.

## Environment

| Variable | Purpose |
|---|---|
| `NOW_MAX_TCP_FLOWS` | TCP flows per authenticated client session (default `1024`) |
| `NOW_MAX_UDP_FLOWS` | UDP flows per authenticated client session (default `256`) |
| `NOW_QUIC_UDP_QUEUE_BYTES` | Bounded QUIC datagram/reassembly memory |
| `NOW_QUIC_MEMORY_PROFILE` | `memory`, `balanced` (default), or `throughput` flow-control budget |
| `NOW_MAX_PENDING_PAIRS` | Pending split-flow pairs |
| `NOW_FLOW_PAIR_TIMEOUT` | Split-flow pairing deadline |
| `NOW_FLOW_SETUP_TIMEOUT` | Client flow setup deadline |
| `NOW_TCP_DATA_BUF_SIZE` | TCP relay buffer |
| `NOW_UDP_DATA_BUF_SIZE` | UDP receive buffer |
| `NOW_TCP_DIAL_TIMEOUT`, `NOW_UDP_DIAL_TIMEOUT` | Target dial deadlines |
| `NOW_TCP_READ_TIMEOUT`, `NOW_UDP_IDLE_TIMEOUT` | Relay idle/half-close deadlines |
| `NOW_HANDSHAKE_TIMEOUT` | TLS, authentication, and request phase deadline |
| `NOW_REPORT_INTERVAL` | Local status-report interval |
| `NOW_TELEMETRY_INTERVAL` | TUI sample period (`250ms..60s`) |
| `NOW_SERVICE_COOLDOWN` | Transport reconnect retry delay |
| `NOW_SHUTDOWN_TIMEOUT` | Graceful shutdown deadline |
| `NOW_RELOAD_INTERVAL` | Minimum PEM certificate reload interval |

Mux limits are library defaults with strict validation: 512 KiB per stream and
connection, 256 active streams per Mux, and 512 queued frame slots. Payload in
the queue is also charged against the 512 KiB connection window, so slot capacity
does not multiply the byte bound. The application uses a 12-flow shard density
and retires fully idle shards after 30 seconds. `NOW_MAX_TCP_FLOWS` is the hard
per-session logical TCP limit shared by TLS and QUIC. `NOW_MAX_UDP_FLOWS` is the
corresponding UDP limit shared by UoT and QUIC DATAGRAM. Excess flows fail
without waiting for capacity. QUIC internally admits the sum of both limits as
bidirectional streams; this derived capacity has no separate setting.

Portal and Vector use the same QUIC profile regardless of ALPN or the client
Mux setting.
The stream/connection/send windows are respectively 4/8/8 MiB for `memory`,
8/16/16 MiB for `balanced`, and 16/32/32 MiB for `throughput`. These are
flow-control ceilings, not eager allocations. Larger windows are useful only
when the required bandwidth-delay product justifies their in-flight memory.
