# Security Model

Nowhere separates transport confidentiality, server identity, shared-key
authorization, and resource admission. These controls work together and should
be configured as distinct trust boundaries.

## Transport Security

TLS/TCP and QUIC require TLS 1.3. Plaintext carriers, application 0-RTT data,
and half-RTT server data are not accepted.

TLS protects carrier confidentiality and integrity. Certificate verification
establishes Portal identity. Exporter-bound shared-key authentication then
authorizes the physical connection to join a logical session and request target
connections.

## Shared-Key Authentication

The shared key is read from the URL username and never sent on the wire. Portal
and client derive an authentication key once, then combine it with the current
TLS exporter, transport domain, and session ID to produce a 16-byte tag.

The exporter binds the tag to one physical TLS or QUIC connection. A captured
32-byte authentication frame therefore cannot authenticate another connection.
Tags are compared in constant time.

Use a high-entropy shared key. Anyone with the key can request arbitrary target
connections because Nowhere does not provide per-user accounts or target
allowlists.

## Certificate Policy

Portal `tls=1` creates a new in-memory self-signed certificate at startup. It
provides encryption but no stable public identity.

Portal `tls=2` loads a PEM certificate chain and private key and supports safe
reload while retaining the last valid certificate.

Vector trust behavior is explicit:

- `sni=<name>` loads system roots and verifies the certificate chain and name.
- Empty, omitted, or `sni=none` disables certificate verification.
- A verification failure closes the carrier and does not fall back to an
  unverified policy.

Exporter authentication does not replace certificate verification. Without
certificate verification, an active intermediary that knows or obtains the
shared key can impersonate Portal.

## Authentication Boundary

Portal never dials a target before authentication succeeds. Authentication
failures wait for the common authentication deadline and expose only a generic
network outcome; detailed diagnostics stay in local logs.

Before QUIC authentication, Portal requires Retry, applies global and
source-prefix admission limits, allows one bidirectional stream, grants
conservative receive credit, and discards all DATAGRAMs.

After authentication, Portal raises the normal stream and receive limits and
registers the carrier under the validated session ID.

## Flow and Memory Boundaries

Explicit limits cover:

- authenticated streams and UDP flows;
- pending asymmetric pairs;
- idle TLS lanes;
- UDP queue packets and bytes;
- fragment reassembly slots, bytes, and lifetime;
- target lengths and flow identifiers;
- authentication, setup, dial, idle, and shutdown deadlines.

Decoders validate enum values, reserved bits, identifiers, and lengths before
allocating from network-controlled input. DATA for unknown flows or flows that
have not reached READY is dropped.

## SOCKS5 Boundaries

Vector ties UDP ASSOCIATE traffic to the TCP control peer, rejects SOCKS5 UDP
fragments, limits target flows globally, and closes target state when the
control connection ends.

Wildcard listeners such as `socks=:1080`, `0.0.0.0`, or `[::]` expose Vector to
other hosts. Protect them with RFC1929 credentials and firewall rules.

Portal outbound SOCKS failures never fall back to direct dialing. When proxying
is configured, domain targets remain unresolved until they reach the proxy.

## Deployment Checklist

- Use `tls=2` and verified Vector `sni` for public deployments.
- Restrict command URL, certificate, and private-key access.
- Use independent high-entropy shared and SOCKS credentials.
- Enable only the required Portal listener transports.
- Keep wildcard SOCKS listeners behind authentication and firewall policy.
- Monitor CHECK_POINT, LINK_STATUS, authentication failures, and restarts.
- Treat DEBUG access paths as sensitive operational metadata.
