# Interoperability

## ALPN contract

Portal and Vector advertise one exact TLS 1.3 ALPN. The default is `now/1`;
`alpn=<value>` selects another nonempty value up to 255 bytes. Peers must use
the same value for TLS/TCP and QUIC/UDP. ALPN does not select a protocol version
or enable Mux.

## TLS lane contract

Vector `mux=0` opens one authenticated TLS connection per Flow. Vector `mux=1`
opens marked Mux connections and assigns logical streams to dynamic Shards.

Portal `mux=0` accepts dedicated lanes. Portal `mux=1` accepts only marked Mux
connections. Both modes are valid 1.8 configurations. After the 32-byte
authentication frame:

- `0xff` identifies a Mux connection;
- in `mux=0`, every other valid byte remains the first byte of a dedicated FlowHeader;
- in `mux=1`, every other byte is rejected.

The marker cannot collide with a valid FlowHeader. This also lets a 1.8 Portal
with the default `mux=0` accept 1.7 clients that do not have a `mux` option.
Authenticated dedicated connections have 40 seconds to provide their first
FlowHeader byte, covering the 30-second warm-lane lifetime used by 1.7 Vector.
A Mux client is rejected safely by a Portal that has not enabled Mux.

## Runtime contract

Mux Shards open lazily at 12 active flows, select the least-loaded live Shard,
and close after 30 seconds fully idle. Dedicated lanes and Mux streams use the
same authentication, FlowHeader, Target, setup result, pairing and limits.
QUIC behavior is independent from the Mux setting.

Peers must also use matching credentials and reachable carrier families. A
Portal with `next=` uses its configured ALPN and Mux mode for the next hop.
