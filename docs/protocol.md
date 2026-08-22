# Nowhere Wire Protocol

This is the authoritative specification for the Nowhere wire protocol. Integers are unsigned and network byte order unless stated otherwise.

## 1. Transport configuration

Every TLS/TCP and QUIC connection negotiates one configured ALPN value. The
default is `now/1`; deployments may use any nonempty value up to 255 bytes.
Peers MUST configure the same exact value. ALPN has no Mux or version semantics.

| `mux` | TLS/TCP data plane | QUIC data plane |
|---|---|---|
| `0` | One authenticated connection per flow lane | Native streams and DATAGRAMs |
| `1` | Authenticated Mux connections | Native streams and DATAGRAMs |

Vector `mux=1` originates Mux connections. Portal `mux=1` accepts only Mux
connections. Portal `mux=0` accepts only dedicated connections. Both TLS modes
are current 1.8 behavior; Mux never wraps QUIC.

## 2. Connection authentication

Every physical TLS or QUIC connection starts with a fixed 32-byte authentication frame on its first byte stream:

| Offset | Size | Field |
|---:|---:|---|
| 0 | 16 | random logical `session_id` |
| 16 | 16 | truncated HMAC-SHA256 tag |

The authentication key is derived from the configured shared key with HKDF-SHA256. The tag covers a carrier domain byte (`1` TLS/TCP, `2` QUIC), the TLS exporter, and `session_id`. The exporter label is `EXPORTER-Nowhere-Auth`.

Authentication is connection-bound. A captured frame cannot authenticate another TLS or QUIC connection.

## 3. Optional TLS Mux

A Mux connection carries the byte `0xff` immediately after its authentication
frame, followed by Mux frames. The marker explicitly opens Mux mode and adds no
request-response exchange. `0xff` cannot begin a valid FlowHeader because its
role bits are reserved. A Mux-enabled Portal requires this marker; any other
first byte is rejected. A dedicated Portal rejects the marker and treats any
other first byte as the first byte of a FlowHeader.

Each Mux frame has an 8-byte header:

| Offset | Size | Field |
|---:|---:|---|
| 0 | 1 | `kind` |
| 1 | 1 | `flags` |
| 2 | 2 | `value` |
| 4 | 4 | `flow_id` |

Kinds:

| Value | Name | Meaning of `value` |
|---:|---|---|
| `0x01` | STREAM | following payload length |
| `0x02` | WINDOW | returned credit; no payload |
| `0x03` | DATAGRAM | following payload length; reserved by the runtime |

STREAM flags are `SYN=0x01`, `FIN=0x02`, and `RST=0x04`. Other bits are invalid. `flow_id=0` is invalid for STREAM and DATAGRAM; WINDOW uses `flow_id=0` for connection credit and a nonzero ID for stream credit.

`SYN` creates the stream before any optional payload is delivered. `FIN` half-closes the sender. `RST` must be the only flag and have zero payload.

The runtime limits STREAM data chunks to 32 KiB. It enforces two independent receive windows:

- a per-stream window;
- a connection-wide window shared by all streams.

The receiver returns both credits only as the application consumes bytes. A
peer exceeding either window or returning excess credit causes the entire Mux
carrier to close. Carrier close immediately fails all streams and releases
their queued payload.

Closing a logical stream is idempotent. A terminal STREAM frame or stream-local
WINDOW that crosses final local cleanup is ignored; DATA for an unknown flow is
still a carrier protocol error. This prevents normal full-duplex close races from
turning one stream shutdown into a carrier-wide failure.

The application runtime stripes active flows across lazily opened Mux
connections. A new flow uses the least-loaded connection. When every live
connection carries 12 active flows, the runtime opens another connection.
A connection with no active flow closes after 30 seconds; activity during that
interval restarts the idle deadline. Sharding is not negotiated on the wire and
does not change frame encoding; every physical connection is an independent
authenticated Mux carrier.

One authenticated client session admits at most 1,024 concurrent logical TCP
flows and 256 concurrent logical UDP flows by default, including pending flows
and regardless of their TLS/QUIC carrier combination. A flow is full-duplex
and counts once. Portal rejects a new flow with `FlowLimit` at its type's limit;
it does not wait for capacity.

A QUIC-carried TCP flow owns one reliable bidirectional data stream. A
QUIC-carried UDP flow owns one reliable bidirectional control stream while its
payload remains in DATAGRAM frames. The authenticated QUIC bidirectional stream
ceiling is therefore the sum of the configured TCP and UDP flow limits, 1,280
by default.

TLS-carried UDP uses the UoT packet codec in section 7, either directly on a
dedicated lane or inside a Mux STREAM. QUIC does not use the Mux header.

## 4. Flow header

Every logical lane begins with a 5-byte FlowHeader:

| Offset | Size | Field |
|---:|---:|---|
| 0 | 1 | packed role, kind, carriers, and hop budget |
| 1 | 4 | nonzero `flow_id` |

Roles are OPEN (uplink half), ATTACH (downlink half), and DUPLEX. Kinds are TCP and UDP. Carrier values describe TLS/TCP or QUIC. OPEN and DUPLEX are followed by a Target; ATTACH is not.

When both directions use the same carrier, the runtime sends one DUPLEX lane;
for TLS this is one dedicated lane in `mux=0` or one logical stream on a Mux carrier in `mux=1`, and
for QUIC it is one bidirectional stream. OPEN and ATTACH are used when uplink and downlink use
different carrier kinds. Portal pairs those halves by `(session_id, flow_id)`
under bounded admission and a finite deadline. The physical carrier
must match the direction declared by the header.

## 5. Target

Target encoding matches SOCKS5 address encoding:

| Type | Encoding |
|---|---|
| IPv4 | `0x01`, 4 address bytes, 2-byte port |
| domain | `0x03`, 1-byte length, name bytes, 2-byte port |
| IPv6 | `0x04`, 16 address bytes, 2-byte port |

Domains must be nonempty safe ASCII wire names. Port zero is invalid.

## 6. Setup result

Portal returns one byte on the selected downlink before payload relay. Zero means READY; nonzero values are typed setup failures. Payload MUST NOT be sent before READY.

## 7. UDP planes

### TLS UoT

Each packet is:

| Size | Field |
|---:|---|
| 2 | payload length |
| variable | payload |

Packets are carried sequentially within the flow's byte stream. Backpressure is therefore identical to TCP stream backpressure.

### QUIC DATAGRAM

QUIC UDP uses a 5-byte flow/data header. Packets larger than the connection datagram size are fragmented with a 13-byte fragment header and reassembled under bounded slot, byte, and TTL limits. A CLOSE datagram removes the route. Unknown, pre-authentication, or pre-READY datagrams are discarded rather than retained.

## 8. Failure semantics

- TLS Mux connection loss fails only the streams assigned to that Mux shard.
- A dedicated TLS lane loss fails that lane.
- QUIC connection loss fails its streams and datagram routes.
- TCP applications reconnect according to their own policy.
- UDP during an unavailable carrier is dropped or fails at the active socket boundary.

Flow state, queued payload, and target sockets are scoped to the active carrier
and are released when that carrier closes.

## 9. Resource invariants

Implementations MUST bound unauthenticated connections, flow IDs, pending pairs, stream and connection windows, outbound frame queues, UDP flows, datagram bytes, and fragment slots. Decoders MUST reject reserved flags, zero IDs where forbidden, invalid lengths, excess credit, and inconsistent fragment metadata.
