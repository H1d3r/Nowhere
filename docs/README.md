# Documentation

The documentation has one source of truth for each concern:

| Need | Document |
|---|---|
| Run a local Portal and Vector | [Quick start](quick-start.md) |
| Choose and operate a supported platform | [Platforms](platforms.md) |
| Understand URL and environment options | [Configuration](configuration.md) |
| Implement or inspect the wire format | [Protocol](protocol.md) |
| Deploy and observe the processes | [Operations](operations.md) |
| Review authentication and memory bounds | [Security](security.md) |
| Understand ALPN and peer interoperability | [Interoperability](compatibility.md) |
| Implement another client or integration | [Integrations](integrations.md) |

`protocol.md` is normative. Portal and Vector share one internal bounded TLS
Mux engine.

Portal and Vector have the same transport behavior on Linux, macOS, and Windows. Platform-specific packaging, process control, filesystem paths, and telemetry availability are documented separately instead of being mixed into the protocol.

## Protocol summary

| Mux setting | TLS/TCP | QUIC/UDP | Failure scope |
|---|---|---|---|
| `mux=0` | Dedicated lane per flow | Native streams/datagrams | The dedicated flow closes with the carrier |
| `mux=1` | Shared bounded Mux | Native streams/datagrams | Assigned flows close with the carrier |

ALPN defaults to `now/1` and is configurable independently from Mux. Peers use
the same exact ALPN. All four uplink/downlink carrier combinations use the same
FlowHeader, Target, pairing, and relay semantics.
