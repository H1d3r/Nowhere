# Operations

## Health model

Portal and Vector expose lifecycle, logical flow counts, TLS/QUIC carrier
counts, and traffic totals through the local TUI on every supported platform.
Linux also reports process CPU and RSS. These process-resource fields are
unavailable on macOS and Windows; relay behavior is unchanged.

Run `nowhere` without a URL and select:

- `1` Overview;
- `2` Logs.

## Capacity

The important memory bounds are the 1,024 concurrent TCP flows and 256 UDP flows
per authenticated client session, the 512 KiB per-stream and per-Mux receive
windows, 256 streams per Mux, bounded reusable relay-buffer caches, and QUIC UDP
queue/reassembly limits. UoT and QUIC DATAGRAM share the UDP flow limit. TLS
shards enabled by `mux=1` target 12 active flows, use least-loaded placement,
and close after 30 seconds fully idle. Frame queue slots do not bypass byte credit. Windows are
granted as permits and payload is admitted incrementally.

QUIC uses the shared `balanced` memory profile by default for both protocol
versions. Select `throughput` only for high-bandwidth, high-RTT paths after
capacity testing; select `memory` when connection density matters more than a
single flow's bandwidth-delay product.

## Failure behavior

When a physical carrier closes, its logical flows close. SSH, download, and
WebSocket clients reconnect according to their application policy after
Wi-Fi/5G changes, NAT rebuilds, or TCP resets.

## Shutdown

Ctrl+C starts graceful shutdown on Linux, macOS, and Windows. Unix process managers may use SIGINT or SIGTERM. Shutdown stops accepting new work, rejects incomplete pairings, lets established relay tasks drain until `NOW_SHUTDOWN_TIMEOUT`, and then closes remaining carriers. Local telemetry registry files are removed when their server exits; stale entries are also discarded during discovery.

Run Portal and Vector under the platform's normal service manager. The manager should preserve the URL and environment configuration, forward a graceful termination event, restart only after the process exits, and allow the configured shutdown deadline.

## Deployment checks

Functional validation belongs on every deployment platform. Check Portal and Vector startup, TCP CONNECT, UDP ASSOCIATE, all configured carrier combinations, graceful shutdown, and local TUI discovery.
