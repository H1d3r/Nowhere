# Integrations

Nowhere exposes integration contracts through command URLs, the local SOCKS5
listener, and the documented wire protocol. Its Rust modules are internal and
do not provide a separately versioned SDK surface.

## Alternate clients

Implementers should follow [Protocol](protocol.md). QUIC remains native and
never uses TLS Mux framing. Peers advertise the exact configured ALPN. A Mux
TLS connection places the `0xff` marker after authentication; a dedicated lane
places its FlowHeader there instead. Portal accepts both forms on the same TLS
listener and selects the decoder from that first byte.

## Chained Portal

`next=shared-key@host:port` creates the same transport-only client engine used
by Vector. `mux=0|1` selects dedicated or Mux TLS for that upstream client and
defaults to `0`; it has no effect without `next`. Authentication, flow setup,
bounds, and failure semantics remain identical at every hop.
