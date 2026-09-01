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
- every other byte is the first byte of a dedicated FlowHeader.

The marker cannot collide with a valid FlowHeader. Dedicated and marked Mux
connections use the same listener without separate inbound configuration.
An authenticated dedicated connection has 40 seconds to provide its first
FlowHeader byte.

```text
                         first byte after AuthFrame
                                      |
                    +-----------------+-----------------+
                    |                                   |
                  0xff                              any other byte
                    |                                   |
                    v                                   v
          +--------------------+              +--------------------+
          | Mux frame decoder  |              | FlowHeader decoder |
          | shared TLS carrier |              | dedicated TLS lane |
          +--------------------+              +--------------------+
```

Portal dispatches every authenticated TLS connection by its framing:

| Bytes after AuthFrame | Selected form | Result |
|---|---|---|
| Valid FlowHeader | Dedicated TLS | accepted |
| `0xff`, then valid Mux frames | Marked Mux TLS | accepted |
| Unmarked Mux bytes | Invalid FlowHeader | rejected |

The `0xff` byte is the Mux mode marker. It is always present on a Mux carrier
and never appears on a dedicated lane.

## Runtime contract

Mux Shards open lazily at 4 active flows, select the least-loaded live Shard,
and close after 30 seconds fully idle. Dedicated lanes and Mux streams use the
same authentication, FlowHeader, Target, setup result, pairing and limits.
QUIC behavior is independent from the client Mux setting.

Peers must also use matching credentials and reachable carrier families. A
Portal with `next=` uses its configured ALPN and the same `tcp|udp|mix` policy
as Vector for the next hop. Mix resolves locally before transmission, and the
peer receives a standard TT, TQ, QT, or QQ FlowHeader. Portal compatibility is
therefore independent of whether the client URL uses a fixed or mixed policy.
The upstream Mux selection defaults to `0`, is ignored without an enabled
`next`, and canonicalizes to `0` for a fixed `udp/udp` route.

Interoperability tests exercise both peer roles: one endpoint as Portal and the
other as client. The complete 3×3 `up`/`down` policy matrix covers all four
concrete routes and all five policies containing `mix`, together with the
default and a custom ALPN, dedicated TLS, and marked Mux.
