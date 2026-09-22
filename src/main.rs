// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

//! Command-line entry point for running a Nowhere Portal or Vector.

use std::env;
use std::io::IsTerminal;

use anyhow::{Context, Result, bail};
use nowhere::common::{LogLevel, Logger, query_first, validate_endpoint_url_input};
use nowhere::portal::Portal;
use nowhere::vector::Vector;
use url::{ParseError, Url};

const VERSION: &str = env!("CARGO_PKG_VERSION");
const HELP_TEXT: &str = "\
Usage:
  nowhere
  nowhere tui
  nowhere <portal-url>
  nowhere <vector-url>
  nowhere -h | --help
  nowhere -v | --version

Commands:
  tui             Open the read-only multi-instance TUI.
  <portal-url>    Run a Portal relay service.
  <vector-url>    Run a Vector SOCKS5 client.

URL forms:
  portal://<key>@<listen-host>:<port>[?<options>]
  portal://<key>@<listen-host>/<carrier>:<port>[/<carrier>:<port>][?<options>]
  vector://<key>@<portal-host>:<port>?socks=<listener>[&<options>]
  vector://<key>@<portal-host>/<carrier>:<port>[/<carrier>:<port>]?socks=...

Endpoint syntax:
  host:port                   Use TCP and UDP on the same port.
  tcp, udp                    Use any address family.
  tcp4, udp4 / tcp6, udp6     Restrict a carrier to IPv4 or IPv6.
  *                           Portal wildcard; clients require a concrete host.

Examples:
  nowhere 'portal://secret@*:2000'
  nowhere 'portal://secret@*/tcp:2006/udp:2017?morph=1'
  nowhere 'vector://secret@relay.example:2000?socks=127.0.0.1:1080'
  nowhere 'vector://secret@relay.example/tcp:2006/udp:2017?up=udp&down=tcp&morph=1&socks=:1080'

Common options:
  morph=0|1           Enable keyed wire masking. Default: 0.
  rate=<mbps>         Client-to-target limit. 0 disables it.
  etar=<mbps>         Target-to-client limit. 0 disables it.
  log=<level>         none, debug, info, warn, error, or event. Default: info.

Portal options:
  tls=1|2             Generated certificate or supplied PEM files. Default: 1.
  crt=<path>          PEM certificate chain for tls=2.
  key=<path>          PEM private key for tls=2.
  dial=<ip|auto>      Source IP for outbound connections. Default: auto.
  socks=<proxy>       Outbound SOCKS5 proxy; mutually exclusive with next.
  next=<portal>       Native upstream Portal: key@host or key@host/<carriers>.

Vector options:
  socks=<listener>    Required local SOCKS5 listener: [user:pass@]host:port.

Client route options (Vector and Portal next):
  up=tcp|udp|mix      Upload carrier. Default: the only carrier, otherwise TCP.
  down=tcp|udp|mix    Download carrier. Default: the only carrier, otherwise TCP.
  mux=0|1             Enable TLS multiplexing when TCP is available. Default: 0.
  sni=<name|none>     Verify the Portal certificate for a DNS name.
  pin=<sha256|none>   Pin the Portal certificate SHA-256 fingerprint.

Morph:
  Both peers on each hop must use morph=1 and the same shared key. Morph masks
  the TLS/QUIC wire image; it does not replace transport security. On Portal,
  it applies to both the listener and the native next hop.

Environment:
  NOW_MORPH_TCP_PRELUDE          Client TCP Morph prelude: low7 (default) or full8.
  NOW_TRANSPORT_MEMORY_PROFILE   memory, balanced, or throughput. Default: throughput.

Documentation:
  https://github.com/NodePassProject/Nowhere/tree/main/docs
";

#[tokio::main]
async fn main() {
    if let Err(err) = start(env::args().collect()).await {
        eprintln!("{}", format_start_error(&err));
        std::process::exit(1);
    }
}

fn format_start_error(error: &anyhow::Error) -> String {
    format!("error: {error:#}")
}

async fn start(args: Vec<String>) -> Result<()> {
    if args.len() == 1 {
        return run_tui().await;
    }
    if args.len() > 2 {
        bail!("expected exactly one configuration URL; run 'nowhere --help' for usage");
    }

    match args[1].as_str() {
        "help" | "--help" | "-h" => {
            print_help();
            return Ok(());
        }
        "version" | "--version" | "-v" => {
            println!(
                "nowhere-v{VERSION} {}/{}",
                env::consts::OS,
                env::consts::ARCH
            );
            return Ok(());
        }
        "tui" => return run_tui().await,
        _ => {}
    }

    let command_url = parse_command_url(&args[1]).with_context(|| "invalid configuration URL")?;
    let scheme = command_url.scheme().to_string();
    if !matches!(scheme.as_str(), "portal" | "vector") {
        bail!("invalid configuration URL: scheme must be portal or vector, found {scheme:?}");
    }
    // Startup only needs `log` here. Each role parses its own configuration,
    // including Portal's intentionally ignored upstream options when `next`
    // is disabled.
    let query =
        query_first(&command_url, &["log"]).with_context(|| "invalid configuration URL query")?;
    let logger = init_logger(query.get("log").map(String::as_str))?;

    match scheme.as_str() {
        "portal" => {
            let portal = Portal::new(command_url, logger)?;
            portal.run().await
        }
        "vector" => {
            let vector = Vector::new(command_url, logger)?;
            vector.run().await
        }
        _ => unreachable!("scheme was validated above"),
    }
}

async fn run_tui() -> Result<()> {
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        bail!("main::run_tui: an interactive terminal is required")
    }
    nowhere::tui::run().await
}

fn print_help() {
    println!(
        "nowhere-v{VERSION} {}/{}\n\n{HELP_TEXT}",
        env::consts::OS,
        env::consts::ARCH
    );
}

fn parse_command_url(raw: &str) -> Result<Url> {
    validate_endpoint_url_input(raw, "endpoint")?;
    match Url::parse(raw) {
        Ok(url) => Ok(url),
        Err(ParseError::EmptyHost) => {
            let normalized = normalize_legacy_empty_portal_host(raw)
                .ok_or(ParseError::EmptyHost)
                .and_then(|url| Url::parse(&url))?;
            Ok(normalized)
        }
        Err(err) => Err(err.into()),
    }
}

/// Converts the V1 compact wildcard alias into the canonical V2 host form.
fn normalize_legacy_empty_portal_host(raw: &str) -> Option<String> {
    let prefix = "portal://";
    let rest = raw.strip_prefix(prefix)?;
    let authority_len = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, suffix) = rest.split_at(authority_len);
    let host_port_start = authority.rfind('@').map_or(0, |idx| idx + 1);
    let host_port = &authority[host_port_start..];

    if !host_port.starts_with(':') || host_port.len() == 1 {
        return None;
    }

    let mut normalized = String::with_capacity(raw.len() + 1);
    normalized.push_str(prefix);
    normalized.push_str(&authority[..host_port_start]);
    normalized.push('*');
    normalized.push_str(host_port);
    normalized.push_str(suffix);
    Some(normalized)
}

fn init_logger(level: Option<&str>) -> Result<Logger> {
    let logger = Logger::new(LogLevel::Info, true);
    match level {
        None | Some("info") => {}
        Some("none") => logger.set_log_level(LogLevel::None),
        Some("debug") => {
            logger.set_log_level(LogLevel::Debug);
            logger.debug(format_args!("main::init_logger: log level set to DEBUG"));
        }
        Some("warn") => {
            logger.set_log_level(LogLevel::Warn);
            logger.warn(format_args!("main::init_logger: log level set to WARN"));
        }
        Some("error") => {
            logger.set_log_level(LogLevel::Error);
            logger.error(format_args!("main::init_logger: log level set to ERROR"));
        }
        Some("event") => {
            logger.set_log_level(LogLevel::Event);
            logger.event(format_args!("main::init_logger: log level set to EVENT"));
        }
        Some(value) => {
            bail!("log must be none, debug, info, warn, error, or event; found {value:?}")
        }
    }
    Ok(logger)
}

#[cfg(test)]
#[path = "tests/main.rs"]
mod tests;
