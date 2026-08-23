# Security and Resource Bounds

## Authentication

The shared key is never sent on the wire. A derived HMAC key authenticates a frame bound to the TLS exporter, carrier type, and random session ID. Replaying the frame on another connection fails.

TLS is version 1.3. Deployments may use a certificate pin, normal system-root verification with SNI, or the explicitly configured unverified certificate mode used by generated local certificates.

## Admission

Portal bounds pre-authentication work and applies per-source admission before expanding QUIC stream windows. Flow pairing, logical IDs, UDP routes, datagram queues, and fragment reassembly are separately bounded.

## Mux memory safety

Mux payload remains within the active carrier's bounded outbound queue. A sender
needs both stream and connection credit before a data frame enters that queue.
A receiver charges both windows before delivery and returns credit only after
application consumption. Closing the carrier releases queued payload.

The fixed maximum frame payload is 65,535 bytes and the runtime emits at most
32 KiB per STREAM frame. Malformed kinds, flags, IDs, lengths, window overflow,
and DATA for unknown streams close the carrier. Late terminal and credit frames
for a terminal stream are idempotent.

Default limits are 512 KiB per stream and per Mux connection and 256 active
streams per Mux. With client `mux=1`, Vector or Portal `next` places at most 12
active flows on a shard before opening another, distributes new flows to the
least-loaded shard, and closes a fully idle shard after 30 seconds. One
authenticated inbound Mux carrier is subject to the same fully idle timeout.
One authenticated client session admits at most 1,024 concurrent logical TCP
flows and 256 logical UDP flows across all of its carriers. UoT and QUIC
DATAGRAM flows share the UDP limit.
Local fair credit prevents one stream from monopolizing a shared window. The
finite frame queue has 512 slots, but payload admission is still capped by the
512 KiB byte window; empty SYN/FIN/WINDOW frames cannot turn those slots into
retained application payload. These are credit ceilings rather than eagerly
allocated payload buffers.

Relay scratch buffers use bounded reuse caches: each process retains at most 64
TCP buffers and 32 UDP buffers. A short-lived concurrency spike therefore
cannot leave an unbounded allocator cache behind.

## Local telemetry

The TUI control plane binds only IPv4 loopback and publishes a descriptor in the platform's per-user temporary directory. The client validates the registry identity against the server hello. No shared keys or payload bytes enter telemetry. Unix registry files receive owner-only permissions; every platform also validates the per-user descriptor and server identity before displaying an instance.

## Threat boundary

Nowhere protects traffic on its carrier links. Target-side security, local SOCKS access control, endpoint compromise, denial of service within configured limits, and application reconnection policy remain operational responsibilities.
