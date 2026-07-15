# Nowhere Documentation

> **One port. Two transports. Split directions.**

Nowhere combines a Portal relay with the native Vector SOCKS5 client. Portal
accepts TLS/TCP and QUIC/UDP on one service port, while logical flows can select
their upload and download carriers independently.

## Capability Map

| Capability | Description |
| --- | --- |
| Shared service port | TLS/TCP and QUIC/UDP use the same Portal address and port |
| Directional carriers | `up` and `down` independently select `tcp` or `udp` |
| TCP relay | TLS/TCP lanes or QUIC bidirectional streams |
| UDP relay | UoT over TLS/TCP or QUIC DATAGRAM |
| Native ingress | Vector SOCKS5 CONNECT, UDP ASSOCIATE, and optional RFC1929 |
| Operations | Pools, rate control, limits, telemetry, reload, and graceful shutdown |

## Documents

| Document | Purpose |
| --- | --- |
| [Quick start](quick-start.md) | Build and run a local Portal and Vector |
| [Configuration](configuration.md) | Command URLs, defaults, and environment limits |
| [Operations](operations.md) | Logs, events, pools, reconnect, certificates, and shutdown |
| [Security](security.md) | Trust policy, authentication, limits, and SOCKS exposure |
| [Protocol](protocol.md) | Authentication, flow, target, DATAGRAM, and UoT wire format |
| [Integrations](integrations.md) | Process managers, OpenCtrl, and client compatibility |

## Terminology

- **Portal**: the service accepting encrypted carriers and dialing targets.
- **Vector**: the Rust client exposing a local SOCKS5 ingress.
- **carrier**: TLS/TCP or QUIC/UDP used for one flow direction.
- **bundle**: carriers sharing an authenticated session identity.
- **UoT**: length-prefixed UDP packets carried over a TLS/TCP half.
- **rate**: client-to-target; **etar**: target-to-client.

## Reading Paths

Operators should read Quick Start, Configuration, Security, then Operations.
Client authors should begin with Protocol and the wire-vector tests. Release
maintainers should review the complete documentation set before coordinating an
upgrade.
