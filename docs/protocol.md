# Nowhere Wire Protocol

Nowhere carries TCP and UDP traffic through a transport bundle made of TLS 1.3
over TCP and QUIC over UDP. A logical flow selects its upload and download
carriers independently, while every carrier in the bundle shares one
authenticated session identity.

This document is the normative wire specification. The words **MUST**,
**MUST NOT**, **SHOULD**, and **MAY** are used as protocol requirements.

## 1. Conventions

- One byte is one octet.
- All multibyte integers use network byte order.
- Byte counts in frame diagrams describe the encoded wire length.
- Readers consume fields from left to right and leave following payload bytes
  untouched.
- Reserved bits MUST be zero when sent and MUST be rejected when received.
- Unknown enum values, zero identifiers where forbidden, truncated fields, and
  lengths outside their declared bounds are protocol errors.

TLS/TCP and QUIC both require TLS 1.3. Each carrier advertises one configured
ALPN value; the default is `now/1`. Peers MUST use the same ALPN.

Application data is not accepted as TLS 0-RTT or QUIC 0-RTT data. QUIC flow
authentication starts only after the 1-RTT handshake completes.

### 1.1 Frame Summary

| Frame | Header length | Variable data |
| --- | ---: | --- |
| Authentication | 32 bytes | None |
| Flow | 5 bytes | Optional target and initial payload |
| Target | 7, 19, or `4 + name length` bytes | Domain name |
| Setup result | 1 byte | None |
| QUIC DATA | 5 bytes | UDP payload |
| QUIC FRAGMENT | 13 bytes | Fragment payload |
| QUIC CLOSE | 5 bytes | None |
| UoT packet | 2 bytes | UDP payload |

## 2. Transport Model

### 2.1 Terms

| Term | Meaning |
| --- | --- |
| Portal | The service that authenticates carriers and dials targets |
| Client | A peer that opens logical flows; Vector is the Rust implementation |
| Bundle | Carriers sharing one 16-byte session ID |
| Lane | One TLS/TCP connection or one QUIC bidirectional stream |
| Uplink | Client-to-target direction |
| Downlink | Target-to-client direction |
| Flow | One logical TCP connection or one logical UDP target path |

### 2.2 Bundle Layout

A client generates one random `session_id[16]` for a bundle. The bundle may
contain one active QUIC connection and multiple authenticated TLS/TCP lanes.
Every active or pending flow is identified by:

```text
(session_id[16], flow_id_u32)
```

```text
                         one logical bundle

  Client                                                  Portal
  +------------------+                              +------------------+
  | session_id[16]   |==============================| session table    |
  +------------------+                              +------------------+
       |        |                                          |       |
       |        +---- QUIC connection over UDP ------------+       |
       |                                                           |
       +------------- TLS/TCP lane(s) -----------------------------+

                    shared address and service port
```

At most one QUIC connection is current for a session. When another QUIC
connection authenticates with the same session ID, Portal makes the new
connection current, stops accepting flows from the displaced connection, and
cancels flow state owned by that connection. Flow data is not moved between
physical connections.

### 2.3 Directional Carriers

The `tcp` and `udp` configuration values select physical carriers, not the
proxied application protocol:

| Configuration value | Carrier | TCP flow data | UDP flow data |
| --- | --- | --- | --- |
| `tcp` | TLS 1.3 over TCP | Stream bytes | UoT packets |
| `udp` | QUIC over UDP | Bidirectional stream bytes | QUIC DATAGRAM |

The four direction combinations are:

| Upload | Download | Flow form |
| --- | --- | --- |
| TLS/TCP | TLS/TCP | One duplex TLS lane |
| TLS/TCP | QUIC | TLS uplink plus QUIC downlink |
| QUIC | TLS/TCP | QUIC uplink plus TLS downlink |
| QUIC | QUIC | One duplex QUIC stream and, for UDP, QUIC DATAGRAM |

## 3. Connection Authentication

### 3.1 Shared-Key Derivation

The URL username is strictly percent-decoded into the shared key. Its length
MUST be `1..255` bytes. A URL password component is invalid.

Implementations derive the connection-independent authentication key once:

```text
salt      = SHA-256(ASCII("nowhere/now/1/auth-root"))
auth_root = HKDF-Extract-SHA256(salt, shared_key)
auth_key  = HKDF-Expand-SHA256(auth_root, ASCII("authentication"), 32)
```

### 3.2 TLS Exporter

After the physical TLS or QUIC handshake, both peers export 32 bytes:

```text
exporter = TLS-Exporter(
  label   = ASCII("EXPORTER-Nowhere-Auth"),
  context = present empty byte string,
  length  = 32
)
```

The transport domain byte is:

| Carrier | Value |
| --- | --- |
| TLS/TCP | `0x01` |
| QUIC | `0x02` |

The authentication tag is:

```text
tag = HMAC-SHA256(
  auth_key,
  transport_u8 || exporter[32] || session_id[16]
)[0..16]
```

### 3.3 Authentication Frame

```text
+--------------------------+--------------------------+
| SESSION_ID               | AUTH_TAG                 |
+--------------------------+--------------------------+
| 16 bytes                 | 16 bytes                 |
+--------------------------+--------------------------+
```

Total length: 32 bytes.

The receiver MUST read exactly 32 bytes, recompute the tag with the exporter
of the current physical connection, and compare the 16-byte tag in constant
time. The session ID is returned only after validation succeeds.

Authentication is the resource boundary:

1. Portal MUST NOT dial a target before authentication succeeds.
2. QUIC permits only the authentication stream and conservative receive credit
   before validation.
3. QUIC DATAGRAMs received before validation are discarded.
4. Authentication failures expose no detailed protocol response to the peer.

Because the tag includes the TLS exporter, a captured frame cannot authenticate
another physical connection.

## 4. Flow Header

Every flow begins with a five-byte header:

```text
+----------+--------------------------+
| FLAGS    | FLOW_ID                  |
+----------+--------------------------+
| 1 byte   | 4 bytes                  |
+----------+--------------------------+
```

`FLOW_ID` is an unsigned 32-bit integer in network byte order. It MUST be
nonzero and unique among active and pending flows in the session. A client
allocates IDs monotonically, skips zero, and does not reuse an ID until its
flow state has been released.

### 4.1 Flags

```text
  7       5   4       3       2       1       0
+-----------+-------+-------+-------+---------------+
| RESERVED  | DOWN  | UP    | KIND  | ROLE          |
+-----------+-------+-------+-------+---------------+
| 3 bits    | 1 bit | 1 bit | 1 bit | 2 bits        |
+-----------+-------+-------+-------+---------------+
```

| Field | Value | Meaning |
| --- | --- | --- |
| ROLE | `0` | DUPLEX |
| ROLE | `1` | OPEN |
| ROLE | `2` | ATTACH |
| ROLE | `3` | Invalid |
| KIND | `0` | TCP flow |
| KIND | `1` | UDP flow |
| UP | `0` | TLS/TCP uplink |
| UP | `1` | QUIC uplink |
| DOWN | `0` | TLS/TCP downlink |
| DOWN | `1` | QUIC downlink |

### 4.2 Roles

| Role | Carrier rule | Current carrier | Target |
| --- | --- | --- | --- |
| DUPLEX | Upload equals download | Matches both directions | Required |
| OPEN | Upload differs from download | Matches upload | Required |
| ATTACH | Upload differs from download | Matches download | Absent |

DUPLEX immediately describes both halves of a symmetric flow. OPEN creates the
uplink half of a split flow. ATTACH supplies the matching downlink half. OPEN
and ATTACH carry identical kind and carrier metadata and may arrive in either
order.

## 5. Target Address

DUPLEX and OPEN are followed by one binary target. ATTACH is followed by no
target bytes. The address type values match SOCKS5 ATYP values.

### 5.1 IPv4

```text
+----------+------------------+------------+
| ATYP     | IPv4 ADDRESS     | PORT       |
+----------+------------------+------------+
| 1 byte   | 4 bytes          | 2 bytes    |
+----------+------------------+------------+
| 0x01     | network octets   | u16        |
+----------+------------------+------------+
```

Total length: 7 bytes.

### 5.2 Domain

```text
+----------+------------+----------------------+------------+
| ATYP     | NAME_LEN   | DOMAIN               | PORT       |
+----------+------------+----------------------+------------+
| 1 byte   | 1 byte     | NAME_LEN bytes       | 2 bytes    |
+----------+------------+----------------------+------------+
| 0x03     | 1..253     | ASCII/IDNA hostname  | u16        |
+----------+------------+----------------------+------------+
```

Total length: `4 + NAME_LEN` bytes.

The domain is an unresolved ASCII/IDNA wire hostname. It MUST NOT include a
port or IPv6 brackets.

### 5.3 IPv6

```text
+----------+-------------------------------+------------+
| ATYP     | IPv6 ADDRESS                  | PORT       |
+----------+-------------------------------+------------+
| 1 byte   | 16 bytes                      | 2 bytes    |
+----------+-------------------------------+------------+
| 0x04     | network octets                | u16        |
+----------+-------------------------------+------------+
```

Total length: 19 bytes.

For every target form, port zero is invalid. Empty domains, non-ASCII domain
bytes, unknown address types, and truncated addresses MUST be rejected before
dialing.

## 6. Setup Result

The selected downlink receives one result byte before application data:

```text
+------------------+
| RESULT           |
+------------------+
| 1 byte           |
+------------------+
```

| Value | Name | Meaning |
| --- | --- | --- |
| `0` | READY | Flow is established |
| `1` | INVALID_REQUEST | Header, role, carrier, or target is invalid |
| `2` | METADATA_CONFLICT | OPEN and ATTACH metadata differ |
| `3` | PAIR_TIMEOUT | The matching split half did not arrive |
| `4` | FLOW_LIMIT | A flow or pending-pair limit was reached |
| `5` | DIAL_FAILED | Portal could not establish the target path |
| `6` | SESSION_REPLACED | The owning session carrier was superseded |
| `7` | INTERNAL_ERROR | Setup failed for an internal reason |

Values outside `0..7` are protocol errors. A rejection closes the flow. READY
transitions the flow into its data mode.

For a split flow, only the selected downlink receives the result. The uplink
does not receive a separate result; the downlink result is authoritative for
the complete logical flow.

## 7. Stream Opening Envelope

The complete stream-side opening sequence is:

```text
+----------------+--------------+----------------+--------------------+
| AUTH           | FLOW HEADER  | TARGET         | INITIAL PAYLOAD    |
+----------------+--------------+----------------+--------------------+
| 0 or 32 bytes  | 5 bytes      | 0 or variable  | 0 or more bytes    |
+----------------+--------------+----------------+--------------------+
```

AUTH is present on every TLS/TCP connection and on the first QUIC
bidirectional stream. It is absent from later streams on the same authenticated
QUIC connection. TARGET is present for DUPLEX and OPEN and absent for ATTACH.

A client MAY submit AUTH, the flow header, the target, and initial TCP payload
in one application write. A receiver MUST consume only the declared fields and
preserve already-buffered payload.

| Physical path | Opening bytes |
| --- | --- |
| Cold TLS lane | `AUTH || FLOW || optional TARGET || optional payload` |
| Warm TLS lane | `AUTH`, idle wait, then `FLOW || optional TARGET || optional payload` |
| First QUIC stream | `AUTH || optional first FLOW || optional TARGET || optional payload` |
| Later QUIC stream | `FLOW || optional TARGET || optional payload` |

## 8. TCP Flow Lifecycle

After READY, a TCP flow carries raw stream bytes. No per-chunk application
header is added.

### 8.1 Symmetric TCP Flow

```text
Client                         Portal                         Target
  |                              |                              |
  | FLOW(DUPLEX, TCP) + TARGET   |                              |
  |----------------------------->|                              |
  |                              | TCP dial                     |
  |                              |----------------------------->|
  | READY                        |                              |
  |<-----------------------------|                              |
  |============== raw stream bytes in both directions ==========|
  |                              |                              |
```

The same TLS lane or QUIC bidirectional stream carries both directions.

### 8.2 Split TCP Flow

```text
Client uplink                  Portal                    Client downlink
     |                           |                              |
     | FLOW(OPEN, TCP) + TARGET  |                              |
     |-------------------------->|                              |
     |                           | FLOW(ATTACH, TCP)            |
     |                           |<-----------------------------|
     |                           |                              |
     |                           | READY                        |
     |                           |----------------------------->|
     |                           |                              |
     | client -> target bytes    |                              |
     |==========================>|                              |
     |                           | target -> client bytes       |
     |                           |=============================>|
```

OPEN owns the client-to-target stream half. ATTACH owns the target-to-client
stream half. Portal pairs them by session ID and flow ID, validates identical
metadata, dials the target, and sends READY on the downlink.

Clean stream EOF closes the sending half. Relay state is released when both
directions complete, setup is rejected, the session is cancelled, or an
operational deadline expires.

## 9. UDP Flow Lifecycle

UDP uses the same flow header, target, pairing rules, and setup result as TCP.
After READY, each direction enters the packet mode selected by its carrier:

| Carrier | UDP packet mode | Flow close signal |
| --- | --- | --- |
| TLS/TCP | UoT length-prefixed packets | Clean stream EOF |
| QUIC | QUIC DATAGRAM frames | CLOSE DATAGRAM |

```text
Client                         Portal                         Target
  |                              |                              |
  | FLOW(UDP) + optional TARGET  |                              |
  |----------------------------->|                              |
  | READY on selected downlink   |                              |
  |<-----------------------------|                              |
  |                              |                              |
  | UoT packet or QUIC DATA      | UDP datagram                 |
  |----------------------------->|----------------------------->|
  |                              | UDP datagram                 |
  | UoT packet or QUIC DATA      |<-----------------------------|
  |<-----------------------------|                              |
```

The client MUST wait for READY before sending QUIC DATA. Portal drains already
queued DATAGRAM input while the flow is still inactive, then activates the
flow immediately after READY is queued on the downlink. DATA for an unknown or
inactive flow is discarded.

## 10. QUIC DATAGRAM Codec

Every DATAGRAM begins with one flags byte. Bits `0..1` select the frame type;
bits `2..7` are reserved.

| Type | Value | Purpose |
| --- | --- | --- |
| DATA | `0` | One complete UDP packet |
| FRAGMENT | `1` | One fragment of a UDP packet |
| CLOSE | `2` | Release a UDP flow |
| Invalid | `3` | Rejected |

### 10.1 DATA

```text
+----------+--------------------------+----------------------+
| FLAGS    | FLOW_ID                  | PAYLOAD              |
+----------+--------------------------+----------------------+
| 1 byte   | 4 bytes                  | 0..65535 bytes       |
+----------+--------------------------+----------------------+
| type=0   | nonzero u32              | one UDP packet       |
+----------+--------------------------+----------------------+
```

Header length: 5 bytes.

A zero-length UDP packet is valid. DATA MUST be used whenever the complete
frame fits the current QUIC maximum DATAGRAM size.

### 10.2 FRAGMENT

```text
+----------+----------+----------+------------+------------+-----------+------------------+
| FLAGS    | FLOW_ID  | PACKET_ID| FRAG_INDEX | FRAG_COUNT | TOTAL_LEN | FRAGMENT         |
+----------+----------+----------+------------+------------+-----------+------------------+
| 1 byte   | 4 bytes  | 4 bytes  | 1 byte     | 1 byte     | 2 bytes   | variable         |
+----------+----------+----------+------------+------------+-----------+------------------+
| type=1   | u32      | u32      | zero-based | 2..255     | 1..65535  | nonempty         |
+----------+----------+----------+------------+------------+-----------+------------------+
```

Header length: 13 bytes.

`FLOW_ID` and `PACKET_ID` MUST be nonzero. `FRAG_INDEX` is zero-based and MUST
be smaller than `FRAG_COUNT`. All fragments for one packet MUST carry identical
flow ID, packet ID, fragment count, and total length. The concatenated fragment
payloads in index order MUST contain exactly `TOTAL_LEN` bytes.

The sender chooses:

```text
fragment_payload_max = max_datagram_size - 13
fragment_count       = ceil(payload_len / fragment_payload_max)
```

The resulting count MUST be in `2..255`. If the QUIC maximum DATAGRAM size
shrinks and a packet must be planned again, the sender uses a new packet ID.
Packet ID allocation skips zero.

### 10.3 CLOSE

```text
+----------+--------------------------+
| FLAGS    | FLOW_ID                  |
+----------+--------------------------+
| 1 byte   | 4 bytes                  |
+----------+--------------------------+
| type=2   | nonzero u32              |
+----------+--------------------------+
```

Total length: 5 bytes. CLOSE has no payload. Receiving it releases the flow,
target socket, reassembly slots, queued packets, and associated waiters.

### 10.4 Reassembly

Reassembly is keyed by `(flow_id, packet_id)`.

- Identical duplicate fragments are ignored.
- A duplicate with different bytes discards the packet.
- Conflicting count or total-length metadata discards the packet.
- A packet whose retained bytes exceed or do not equal `TOTAL_LEN` is
  discarded.
- Expired or resource-constrained partial packets are discarded.
- Closing a flow releases all of its partial packets.

The Rust implementation keeps Quinn-owned fragment payloads as `Bytes` slices
and allocates the contiguous UDP packet only when every fragment is present.
Portal and Vector each limit reassembly to 64 concurrent slots with a 10-second
lifetime. Reassembly bytes share the configured UDP queue budget.

## 11. UDP over TLS/TCP

After READY, every UoT packet is encoded as:

```text
+----------------+--------------------------+
| PAYLOAD_LEN    | PAYLOAD                  |
+----------------+--------------------------+
| 2 bytes        | PAYLOAD_LEN bytes        |
+----------------+--------------------------+
| u16            | one UDP packet           |
+----------------+--------------------------+
```

Header length: 2 bytes. `PAYLOAD_LEN=0` represents a valid empty UDP packet.
Each frame contains exactly one datagram. Consecutive packets are encoded back
to back without an additional type field.

A clean EOF before the next length field closes the UoT half. EOF after one
length byte or before the declared payload completes is a protocol error.

## 12. Physical Carrier Lifecycles

### 12.1 TLS/TCP

```text
TCP connect
  -> TLS 1.3 handshake
  -> derive exporter
  -> send AUTH[32]
  -> send FLOW now or wait as an authenticated warm lane
  -> receive setup result on the selected downlink
  -> relay one flow half
  -> close; the lane is not reused
```

An authenticated lane with no immediately readable flow bytes may enter the
Portal idle-lane budget. A cold lane carrying AUTH and FLOW together bypasses
that idle budget. A lane carries one complete duplex flow or one half of a
split flow.

### 12.2 QUIC

```text
UDP path
  -> QUIC/TLS 1.3 handshake with Retry
  -> open first bidirectional stream
  -> send AUTH[32] and optionally the first FLOW
  -> Portal expands stream and receive limits
  -> later FLOWs use new bidirectional streams
  -> UDP payloads use DATAGRAM after READY
  -> connection close cancels owned streams and UDP state
```

One QUIC connection multiplexes TCP streams, UDP control streams, and UDP
DATAGRAM payloads for the session. Unidirectional QUIC streams are not used.

## 13. Validation and Resource Rules

An implementation MUST validate the smallest enclosing header before reading
variable-length data. Network-provided lengths MUST be checked before memory is
reserved.

Portal applies explicit bounds to:

- unauthenticated connections globally and by source prefix;
- authenticated QUIC streams;
- active UDP flows;
- pending split pairs;
- idle TLS lanes;
- UDP queue bytes and packets;
- reassembly slots, bytes, and lifetime;
- target length, authentication, setup, and idle time.

Queue pressure, unknown UDP flow IDs, DATA before READY, expired fragments, and
reassembly conflicts are handled by dropping the affected packet or flow state.
They do not create unbounded queues or trigger allocation from unchecked input.
