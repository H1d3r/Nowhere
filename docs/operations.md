# Operations Guide

Portal and Vector expose their effective configuration, transport state, flow
counts, traffic counters, and lifecycle through local logs. Credentials are
never included in effective URLs or access records.

## Startup Output

Both commands validate selected values before opening listeners. Unknown
parameters and later duplicates are ignored; missing optional parameters use
their defaults.

Portal prints:

```text
net -> tls -> alpn -> rate -> etar -> dial -> socks
```

Vector prints:

```text
up -> down -> pool -> sni -> alpn -> rate -> etar -> socks
```

Vector records `sni=none` whenever certificate verification is disabled and
reports `pool=0` for every carrier pair except `tcp/tcp`.

## Logs and Telemetry

Available levels are `none`, `debug`, `info`, `warn`, `error`, and `event`.

EVENT emits machine-readable checkpoints:

```text
CHECK_POINT|MODE=0|PING=0ms|POOL=5|TCPS=0|UDPS=0|TCPRX=0|TCPTX=0|UDPRX=0|UDPTX=0
```

Portal MODE values are `0=mix`, `1=tcp`, and `2=udp`. Vector MODE values are
`0=tcp/tcp`, `1=tcp/udp`, `2=udp/tcp`, and `3=udp/udp`.

DEBUG additionally emits carrier state:

```text
LINK_STATUS|TCP=0|UDP=0|PAIRS=0|UPTCP=0|UPUDP=0|DOWNTCP=0|DOWNUDP=0
```

Access paths use matching `starting` and `complete` messages. They include the
selected upload and download carriers plus client, relay, and target endpoints,
but never shared keys or SOCKS passwords.

## TLS Warm Lanes

Vector uses a warm pool only for `tcp/tcp`. Each prepared lane completes TCP,
TLS, exporter derivation, and the 32-byte authentication exchange before it is
placed in the idle set.

An acquired lane is single-use. Consumed, closed, unhealthy, or expired lanes
are removed and replenished in the background. Portal independently limits the
number of authenticated idle lanes, so client and server pool controls protect
different resources.

## QUIC Sessions and Recovery

Vector shares one QUIC connection across eligible TCP streams and UDP flows.
When Portal is unavailable, the SOCKS listener remains active while affected
requests fail cleanly. Later requests trigger reconnect after
`NOW_SERVICE_COOLDOWN`.

The logical session ID remains stable across QUIC reconnects. Portal admits one
current QUIC carrier for that session and cancels state owned by a displaced
connection instead of moving live flows between connections.

## Limits and Rate Control

`rate` applies client-to-target and `etar` applies target-to-client. Portal and
Vector enforce their configured limits independently; the tighter side bounds
the complete path.

Increase stream, flow, queue, or pair limits only after measuring CPU, memory,
queue pressure, and target behavior. Queue overload, unknown UDP flows, DATA
before READY, and expired fragments are dropped instead of accumulating
unbounded state.

## Certificates

Portal `tls=2` validates PEM files at startup and checks for replacement files
no more often than `NOW_RELOAD_INTERVAL`. A reload failure keeps the last valid
certificate active and emits an error.

Vector loads system roots for verified `sni` connections. A root-store,
certificate-chain, or name error fails the carrier. There is no automatic
fallback from verified to unverified TLS.

## Graceful Shutdown

On Ctrl-C, listeners and reconnect loops stop, QUIC endpoints send close,
pending pairs are cancelled, and active tasks drain for
`NOW_SHUTDOWN_TIMEOUT`. At the deadline, remaining tasks are aborted and pool,
flow, queue, and rate-limit state is released.
