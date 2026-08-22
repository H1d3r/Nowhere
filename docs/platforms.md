# Platforms

Nowhere uses one codebase and one configuration model on Linux, macOS, and Windows. Portal and Vector support the same TLS 1.3, QUIC, SOCKS5, split-carrier, authentication, flow-control, and failure semantics on every supported platform.

## Release targets

| Platform | Architecture and toolchain | Archive |
|---|---|---|
| Linux | x86-64 and AArch64, GNU libc | `.tar.gz` |
| Linux | x86-64 and AArch64, musl | `.tar.gz` |
| macOS | Apple Silicon | `.tar.gz` |
| Windows | x86-64 MSVC | `.zip` |

The executable is named `nowhere` on Linux and macOS and `nowhere.exe` on Windows. Source builds use the normal Rust workflow on every platform:

```text
cargo build --release --locked
cargo test --all-targets
```

## Command lines

Bourne-compatible shells and PowerShell accept the documented single-quoted URLs:

```text
nowhere 'vector://secret@portal.example:2077?up=tcp&down=udp&socks=127.0.0.1:1080'
```

In Windows Command Prompt, use double quotes and the `.exe` name:

```text
nowhere.exe "vector://secret@portal.example:2077?up=tcp&down=udp&socks=127.0.0.1:1080"
```

Certificate and key values accept native filesystem paths. Relative paths are resolved from the process working directory. Quote a URL whenever a path or query value contains shell-significant characters.

## Process control

Interactive instances stop with Ctrl+C on every platform. Unix process managers may send SIGINT or SIGTERM. Windows services should use a wrapper that forwards a console termination event and allows `NOW_SHUTDOWN_TIMEOUT` to complete.

Local TUI discovery uses a per-user registry in the operating system's temporary directory and a loopback TCP control socket. It does not depend on Unix domain sockets or `/proc`.

## Telemetry

The TUI displays lifecycle, transport counts, flow counts, traffic totals, and
carrier state on every platform. Linux additionally obtains process CPU and
resident-memory samples from `/proc`; those fields are unavailable on macOS and
Windows without affecting relay operation. Cross-platform release confidence
comes from the Linux, macOS, and Windows CI matrix.
