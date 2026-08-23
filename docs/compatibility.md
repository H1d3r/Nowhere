# Interoperability

## ALPN contract

Portal and Vector advertise one exact TLS 1.3 ALPN. The default is `now/1`;
`alpn=<value>` selects another nonempty value up to 255 bytes. Peers must use
the same value for TLS/TCP and QUIC/UDP. ALPN does not select a protocol version
or enable Mux.

## TLS lane contract

Vector `mux=0` opens one authenticated TLS connection per Flow. Vector `mux=1`
opens marked Mux connections and assigns logical streams to dynamic Shards.

Portal accepts both forms on one listener. After the 32-byte authentication
frame:

- `0xff` identifies a Mux connection;
- every other byte remains the first byte of a dedicated FlowHeader.

The marker cannot collide with a valid FlowHeader. Dedicated and marked Mux
connections use the same listener without separate inbound configuration.
An authenticated dedicated connection has 40 seconds to provide its first
FlowHeader byte.

## Runtime contract

Mux Shards open lazily at 12 active flows, select the least-loaded live Shard,
and close after 30 seconds fully idle. Dedicated lanes and Mux streams use the
same authentication, FlowHeader, Target, setup result, pairing and limits.
QUIC behavior is independent from the client Mux setting.

Peers must also use matching credentials and reachable carrier families. A
Portal with `next=` uses its configured ALPN and `mux=0|1` selection for the
next hop. The upstream Mux selection defaults to `0` and is ignored without an
enabled `next`.
