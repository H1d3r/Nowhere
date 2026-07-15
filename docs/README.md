# Nowhere Documentation

> **One port. Two transports. Split directions.**

Nowhere combines TLS/TCP and QUIC/UDP on one Portal service port. TCP and UDP
flows choose their upload and download carriers independently, while the native
Vector client exposes the complete transport model through a local SOCKS5
entry point.

## System Map

| Layer | Responsibility |
| --- | --- |
| Portal | Authenticate carriers, pair directions, dial targets, and relay data |
| Vector | Accept SOCKS5 CONNECT and UDP ASSOCIATE requests and open flows |
| TLS/TCP | Carry TCP stream bytes or length-prefixed UDP packets |
| QUIC/UDP | Carry TCP stream bytes or QUIC DATAGRAM packets |
| Session | Bind physical carriers to one authenticated logical identity |
| Flow | Describe target, payload kind, and both directional carriers |

## Guides

| Document | Purpose |
| --- | --- |
| [Quick Start](quick-start.md) | Build and run Portal and Vector locally |
| [Configuration](configuration.md) | Command URLs, defaults, and runtime limits |
| [Operations](operations.md) | Logs, pools, reconnection, certificates, and shutdown |
| [Security](security.md) | Trust boundaries, authentication, and resource controls |
| [Wire Protocol](protocol.md) | Authentication, flow setup, frame diagrams, and lifecycles |
| [Integrations](integrations.md) | Process managers, SOCKS5, OpenCtrl, and client contracts |

## Terminology

- **Portal** accepts encrypted carriers and opens target connections.
- **Vector** is the Rust client and local SOCKS5 ingress.
- **Uplink** is client to target; **downlink** is target to client.
- **Carrier** is TLS/TCP or QUIC/UDP for one direction.
- **Bundle** is a set of carriers sharing one authenticated session ID.
- **UoT** carries individual UDP packets over a TLS/TCP stream.
- **rate** limits uplink traffic; **etar** limits downlink traffic.

Operators should start with Quick Start, Configuration, Security, and
Operations. Client implementers should start with Wire Protocol and then read
Security and Integrations.
