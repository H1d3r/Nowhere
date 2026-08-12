# Quick Start

## Build

Nowhere uses stable Rust and the 2024 edition and must be built for Linux:

```bash
git clone https://github.com/NodePassProject/Nowhere.git
cd Nowhere
cargo build --release --locked
./target/release/nowhere --version
./target/release/nowhere --help
```

On macOS with Apple Container, the repository provides a pinned Linux check
environment instead:

```bash
./scripts/check-linux.sh
```

## Start a Local Portal

```bash
./target/release/nowhere 'portal://secret@127.0.0.1:2077?log=debug'
```

Portal starts TLS/TCP and QUIC/UDP on port 2077. Its default in-memory
certificate is intended for local operation.

## Start Vector

In another terminal:

```bash
./target/release/nowhere \
  'vector://secret@127.0.0.1:2077?up=udp&down=udp&sni=none&socks=127.0.0.1:1080&log=debug'
```

This exposes a SOCKS5 listener on `127.0.0.1:1080`. `sni=none` deliberately
disables certificate verification for the local in-memory certificate.

Test a TCP request:

```bash
curl --proxy socks5h://127.0.0.1:1080 https://example.com/
```

Applications supporting SOCKS5 UDP ASSOCIATE can use the same listener for
UDP. Vector keeps one idle-timed Nowhere UDP flow per target address.

## Select Directional Carriers

Set upload and download independently:

```text
up=tcp&down=tcp&pool=5
up=tcp&down=udp
up=udp&down=tcp
up=udp&down=udp
```

Split combinations require Portal `net=mix`, which is the default. The warm
pool applies only to `tcp/tcp`; every other pair reports `pool=0`.

## Add a Native Relay Portal

Run the final Portal with its own key:

```bash
nowhere 'portal://origin-key@127.0.0.1:3077'
```

Then run one relay process whose upstream is that Portal:

```bash
nowhere \
  'portal://relay-key@127.0.0.1:2077?next=origin-key@127.0.0.1:3077&up=udp&down=udp'
```

Point Vector at port 2077 with `relay-key`. The relay terminates its incoming
TLS/QUIC connection and opens a native flow to port 3077 without a local
SOCKS5 conversion. Use `tcp/tcp`, `tcp/udp`, `udp/tcp`, or `udp/udp` exactly as
you would on Vector.

## Use a Trusted Certificate

Portal:

```bash
nowhere \
  'portal://secret@:2077?net=mix&tls=2&crt=/etc/nowhere/fullchain.pem&key=/etc/nowhere/privkey.pem'
```

Vector:

```bash
nowhere \
  'vector://secret@relay.example:2077?up=tcp&down=tcp&pool=5&sni=relay.example&socks=127.0.0.1:1080'
```

ALPN defaults to `now/1`. If it is overridden, Portal and Vector must receive
the same nonempty value.

## Stop

Send Ctrl-C, SIGINT, or SIGTERM. Portal rejects unready and new flows while
already-READY relays drain until the single `NOW_SHUTDOWN_TIMEOUT` deadline.
Send a second signal to force shutdown. Vector closes its local and remote work
immediately, bounded by the same timeout.
