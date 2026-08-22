# Integrations

Nowhere exposes integration contracts through command URLs, the local SOCKS5
listener, and the documented wire protocol. Its Rust modules are internal and
do not provide a separately versioned SDK surface.

## Alternate clients

Implementers should follow [Protocol](protocol.md). QUIC remains native and
never uses TLS Mux framing. Peers advertise the exact configured ALPN. A Mux
TLS connection places the `0xff` marker after authentication; a dedicated lane
places its FlowHeader there instead. Portal `mux=1` requires the marker, while
Portal `mux=0` accepts dedicated lanes.

## Chained Portal

`next=shared-key@host:port` creates the same transport-only client used by Vector, so carrier choice, authentication, flow setup, bounds, and failure semantics remain identical at every hop.
