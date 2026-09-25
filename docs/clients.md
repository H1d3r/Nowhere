# Clients and share links

Connect to a Portal with [Anywhere](https://github.com/NodePassProject/Anywhere)
on iOS, iPadOS, and tvOS, or with **Vector**, the SOCKS5 client included in the
Nowhere binary.

| Client | Get started |
| --- | --- |
| Anywhere | [App Store](https://apps.apple.com/us/app/id6758235178) · [Source code](https://github.com/NodePassProject/Anywhere) · [Import guide](https://github.com/NodePassProject/Anywhere#deep-links) |
| Vector | [Quick start](quick-start.md) · [Configuration](configuration.md#vector-url) |

## Link format

Nowhere uses separate URL schemes for client sharing and service configuration:

| Scheme | Purpose | Used by |
| --- | --- | --- |
| `nowhere://` | Share a Portal connection and display name | Anywhere |
| `vector://` | Connect to Portal and expose a local SOCKS5 listener | `nowhere` CLI |
| `portal://` | Configure a server listener and optional forwarding | `nowhere` CLI |

### Client share links

The following format describes Anywhere's current import/export support:

```text
nowhere://KEY@HOST:PORT[?QUERY][#NAME]
nowhere://KEY@HOST/tcp:TCP_PORT[/udp:UDP_PORT][?QUERY][#NAME]
nowhere://KEY@HOST/udp:UDP_PORT[?QUERY][#NAME]
```

`KEY` is the shared Portal key, percent-encoded as URL userinfo; it is not
Base64. Encode reserved characters such as `@`, `:`, `/`, `?`, `#`, and `%`.
The decoded key must contain 1–255 UTF-8 bytes. Use a concrete hostname or IP
address and ports from `1` to `65535`; bracket IPv6 literals, as in
`[2001:db8::1]:2000`. The optional `NAME` is a percent-encoded display name.

The compact `HOST:PORT` form enables both carriers on one port. The explicit
form enables only the listed carriers, each at most once, and has no port
on `HOST`. Both carriers share the same host.

| Query | Values | Default / behavior |
| --- | --- | --- |
| `up` | `tcp`, `udp` | Upload carrier; TCP if available, otherwise UDP |
| `down` | `tcp`, `udp` | Download carrier; TCP if available, otherwise UDP |
| `mux` | `0`, `1` | `0`; `1` enables TLS multiplexing when either direction uses TCP |
| `morph` | `0`, `1` | `0`; must match the Portal's Morph setting |
| `sni` | Server name | Endpoint host when omitted, empty, or `none` |

Each direction must select a carrier declared by the endpoint. Percent-encode
reserved characters in query values. Morph Prelude selection is a local client
setting and is not included in share links.

### Anywhere compatibility

- **No `mix` policy:** `up` and `down` accept only `tcp` or `udp`.
- **No address-family suffixes:** carrier paths accept only `tcp` and `udp`,
  not `tcp4`, `tcp6`, `udp4`, or `udp6`. IPv6 literals such as
  `[2001:db8::1]:2000` are supported; the restriction concerns explicit
  address-family selection through carrier suffixes.
- **No certificate `pin` parameter:** Anywhere's share-link parser does not
  import the CLI's certificate pin setting.

### Examples

**TLS over TCP with multiplexing**

```text
nowhere://change-me@relay.example:2000?up=tcp&down=tcp&mux=1#My%20Portal
```

**QUIC over UDP with Morph** — the Portal must also set `morph=1`.

```text
nowhere://change-me@relay.example:2000?up=udp&down=udp&morph=1#QUIC%20Portal
```

**Separate ports and split directions** — upload over TLS, download over QUIC.

```text
nowhere://change-me@relay.example/tcp:2006/udp:2017?up=tcp&down=udp&sni=relay.example#Split%20Portal
```

Paste a link into Anywhere, or use its deep link to open the import screen:

```text
anywhere://add-proxy?link=nowhere://change-me@relay.example:2000?up=tcp&down=tcp&mux=1#My%20Portal
```

The `add-proxy` wrapper takes everything after `?link=` verbatim; do not
percent-encode the entire inner URL again.

### CLI configuration links

The CLI accepts `portal://` and `vector://`. Vector additionally requires
`socks=` for its local listener; CLI URLs do not accept display-name fragments.
The `sni` default differs: Vector defaults to no certificate verification,
while Anywhere uses the endpoint host as its server name. See
[Security](security.md) for
Vector's verification policy.

Use [Configuration](configuration.md) for the full CLI grammar, listener
options, chaining, and environment variables.
