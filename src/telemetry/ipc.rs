// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

//! Abstract Unix-socket transport and `/proc` discovery.

use std::io;
use std::os::linux::net::SocketAddrExt;
use std::os::unix::net::{
    SocketAddr, UnixListener as StdUnixListener, UnixStream as StdUnixStream,
};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{Semaphore, broadcast};
use tokio_util::sync::CancellationToken;

use super::process::read_start_ticks;
use super::{
    ClientMessage, Hello, MAX_FRAME_SIZE, PROTOCOL_VERSION, ServerMessage, Subscription,
    TelemetryHub,
};

const SOCKET_PREFIX: &str = "@nowhere.v1.";
const MAX_CLIENTS: usize = 16;
const WRITE_TIMEOUT: Duration = Duration::from_secs(2);

/// A socket name validated against the live `/proc/<pid>/stat` incarnation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DiscoveredInstance {
    pub(crate) socket_name: String,
    pub(crate) uid: u32,
    pub(crate) pid: u32,
    pub(crate) start_ticks: u64,
}

/// Publishes one process hub to any number of read-only TUI clients.
pub(crate) struct TelemetryServer {
    listener: UnixListener,
    hub: Arc<TelemetryHub>,
    clients: Arc<Semaphore>,
}

impl TelemetryServer {
    pub(crate) fn bind(hub: Arc<TelemetryHub>) -> Result<Self> {
        if let Some(reason) = hub.unavailable_reason() {
            bail!("telemetry process identity is unavailable: {reason}");
        }
        let name = hub.descriptor().socket_name();
        let address = SocketAddr::from_abstract_name(name.as_bytes())
            .context("telemetry: invalid abstract socket name")?;
        let listener = StdUnixListener::bind_addr(&address)
            .with_context(|| format!("telemetry: failed to bind @{name}"))?;
        listener
            .set_nonblocking(true)
            .context("telemetry: failed to make abstract socket nonblocking")?;
        Ok(Self {
            listener: UnixListener::from_std(listener)
                .context("telemetry: failed to register abstract socket")?,
            hub,
            clients: Arc::new(Semaphore::new(MAX_CLIENTS)),
        })
    }

    pub(crate) async fn run(self, shutdown: CancellationToken) {
        loop {
            let accepted = tokio::select! {
                _ = shutdown.cancelled() => return,
                accepted = self.listener.accept() => accepted,
            };
            let Ok((stream, _)) = accepted else {
                tokio::select! {
                    _ = shutdown.cancelled() => return,
                    _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => {}
                }
                continue;
            };
            let Ok(peer) = stream.peer_cred() else {
                continue;
            };
            let owner = self.hub.descriptor().uid;
            if peer.uid() != owner && peer.uid() != 0 {
                continue;
            }
            let Ok(permit) = Arc::clone(&self.clients).try_acquire_owned() else {
                tokio::spawn(async move {
                    let (_, mut writer) = stream.into_split();
                    let _ = write_frame(
                        &mut writer,
                        &ServerMessage::Error {
                            message: format!("telemetry connection limit reached ({MAX_CLIENTS})"),
                        },
                    )
                    .await;
                });
                continue;
            };
            let hub = Arc::clone(&self.hub);
            let connection_shutdown = shutdown.clone();
            tokio::spawn(async move {
                let _permit = permit;
                let _ = serve_client(stream, hub, connection_shutdown).await;
            });
        }
    }
}

async fn serve_client(
    stream: UnixStream,
    hub: Arc<TelemetryHub>,
    shutdown: CancellationToken,
) -> Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut reader = FrameReader::new(reader);
    let mut lifecycles = hub.lifecycle_receiver();
    let initial_lifecycle = lifecycles.borrow_and_update().clone();
    write_frame(
        &mut writer,
        &ServerMessage::Hello(Hello {
            instance: hub.descriptor().clone(),
            lifecycle: initial_lifecycle.state,
            lifecycle_reason: initial_lifecycle.reason,
        }),
    )
    .await?;

    let mut snapshots = hub.snapshot_receiver();
    let mut events = hub.event_receiver();
    let mut subscription = Subscription::Summary;
    let initial_snapshot = snapshots.borrow_and_update().clone();
    write_frame(&mut writer, &ServerMessage::Snapshot(initial_snapshot)).await?;

    loop {
        tokio::select! {
            _ = shutdown.cancelled() => return Ok(()),
            command = reader.next::<ClientMessage>() => {
                match command? {
                    ClientMessage::Subscribe { subscription: next } => {
                        subscription = next;
                    }
                }
            }
            changed = snapshots.changed() => {
                changed.context("telemetry snapshot source closed")?;
                let snapshot = snapshots.borrow_and_update().clone();
                write_frame(
                    &mut writer,
                    &ServerMessage::Snapshot(snapshot),
                ).await?;
            }
            changed = lifecycles.changed() => {
                changed.context("telemetry lifecycle source closed")?;
                let lifecycle = lifecycles.borrow_and_update().clone();
                write_frame(
                    &mut writer,
                    &ServerMessage::Lifecycle(lifecycle),
                ).await?;
            }
            event = events.recv() => {
                match event {
                    Ok(event) if subscription == Subscription::Detail => {
                        write_frame(&mut writer, &event).await?;
                    }
                    Ok(_) => {}
                    Err(broadcast::error::RecvError::Lagged(missed))
                        if subscription == Subscription::Detail =>
                    {
                        write_frame(
                            &mut writer,
                            &ServerMessage::Gap { missed },
                        ).await?;
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => return Ok(()),
                }
            }
        }
    }
}

/// Scans only the caller's current network namespace.
///
/// Non-root callers skip sockets whose encoded UID differs before connecting.
/// The service repeats this check using `SO_PEERCRED`.
pub(crate) fn discover_instances() -> io::Result<Vec<DiscoveredInstance>> {
    let contents = std::fs::read_to_string("/proc/net/unix")?;
    let current_uid = unsafe { libc::geteuid() };
    let is_root = current_uid == 0;
    let mut found = Vec::new();
    for path in contents
        .lines()
        .filter_map(|line| line.split_ascii_whitespace().last())
        .filter(|path| path.starts_with(SOCKET_PREFIX))
    {
        let Some(instance) = parse_socket_path(path) else {
            continue;
        };
        if !is_root && instance.uid != current_uid {
            continue;
        }
        if read_start_ticks(instance.pid) != Some(instance.start_ticks) {
            continue;
        }
        found.push(instance);
    }
    found.sort_by_key(|instance| (instance.uid, instance.pid, instance.start_ticks));
    found.dedup();
    Ok(found)
}

fn parse_socket_path(path: &str) -> Option<DiscoveredInstance> {
    let mut components = path.strip_prefix('@')?.split('.');
    if components.next()? != "nowhere" || components.next()? != "v1" {
        return None;
    }
    let uid = components.next()?.parse().ok()?;
    let pid = components.next()?.parse().ok()?;
    let start_ticks = components.next()?.parse().ok()?;
    if components.next().is_some() {
        return None;
    }
    Some(DiscoveredInstance {
        socket_name: path.strip_prefix('@')?.to_owned(),
        uid,
        pid,
        start_ticks,
    })
}

pub(crate) struct TelemetryClient {
    hello: Hello,
    reader: TelemetryReader,
    writer: TelemetryWriter,
}

impl TelemetryClient {
    pub(crate) async fn connect(
        discovered: &DiscoveredInstance,
        subscription: Subscription,
    ) -> Result<Self> {
        let address = SocketAddr::from_abstract_name(discovered.socket_name.as_bytes())
            .context("telemetry: invalid discovered abstract socket name")?;
        let stream = StdUnixStream::connect_addr(&address)
            .with_context(|| format!("telemetry: failed to connect @{}", discovered.socket_name))?;
        stream
            .set_nonblocking(true)
            .context("telemetry: failed to make client socket nonblocking")?;
        let stream =
            UnixStream::from_std(stream).context("telemetry: failed to register client socket")?;
        let peer = stream
            .peer_cred()
            .context("telemetry: failed to inspect service peer credentials")?;
        if peer.uid() != discovered.uid {
            bail!(
                "telemetry: service UID mismatch (expected {}, got {})",
                discovered.uid,
                peer.uid()
            );
        }
        let peer_pid = peer
            .pid()
            .and_then(|pid| u32::try_from(pid).ok())
            .context("telemetry: service peer PID is unavailable")?;
        if peer_pid != discovered.pid {
            bail!(
                "telemetry: service PID mismatch (expected {}, got {})",
                discovered.pid,
                peer_pid
            );
        }
        let (reader, mut writer) = stream.into_split();
        let mut reader = FrameReader::new(reader);
        let message = reader.next::<ServerMessage>().await?;
        let hello = match message {
            ServerMessage::Hello(hello) => hello,
            ServerMessage::Error { message } => {
                bail!("telemetry: service rejected connection: {message}")
            }
            _ => bail!("telemetry: service did not begin with hello"),
        };
        validate_hello(&hello, discovered)?;
        write_frame(&mut writer, &ClientMessage::Subscribe { subscription }).await?;
        Ok(Self {
            hello,
            reader: TelemetryReader { inner: reader },
            writer: TelemetryWriter { inner: writer },
        })
    }

    pub(crate) fn hello(&self) -> &Hello {
        &self.hello
    }

    pub(crate) fn into_parts(self) -> (Hello, TelemetryReader, TelemetryWriter) {
        (self.hello, self.reader, self.writer)
    }
}

fn validate_hello(hello: &Hello, discovered: &DiscoveredInstance) -> Result<()> {
    let instance = &hello.instance;
    if instance.protocol_version != PROTOCOL_VERSION
        || instance.uid != discovered.uid
        || instance.pid != discovered.pid
        || instance.start_ticks != discovered.start_ticks
        || instance.socket_name() != discovered.socket_name
    {
        bail!("telemetry: hello identity does not match discovered socket");
    }
    Ok(())
}

pub(crate) struct TelemetryReader {
    inner: FrameReader<OwnedReadHalf>,
}

impl TelemetryReader {
    pub(crate) async fn next_message(&mut self) -> Result<ServerMessage> {
        self.inner.next().await
    }
}

pub(crate) struct TelemetryWriter {
    inner: OwnedWriteHalf,
}

impl TelemetryWriter {
    pub(crate) async fn subscribe(&mut self, subscription: Subscription) -> Result<()> {
        write_frame(&mut self.inner, &ClientMessage::Subscribe { subscription }).await
    }
}

async fn write_frame<W, T>(writer: &mut W, value: &T) -> Result<()>
where
    W: AsyncWrite + Unpin,
    T: Serialize,
{
    let payload = serde_json::to_vec(value).context("telemetry: failed to encode JSON frame")?;
    if payload.len() > MAX_FRAME_SIZE {
        bail!("telemetry: encoded frame exceeds {MAX_FRAME_SIZE} bytes");
    }
    write_payload_with_timeout(writer, &payload, WRITE_TIMEOUT).await
}

async fn write_payload_with_timeout<W>(
    writer: &mut W,
    payload: &[u8],
    timeout: Duration,
) -> Result<()>
where
    W: AsyncWrite + Unpin,
{
    tokio::time::timeout(timeout, write_payload(writer, payload))
        .await
        .context("telemetry: timed out writing frame")?
}

async fn write_payload<W>(writer: &mut W, payload: &[u8]) -> Result<()>
where
    W: AsyncWrite + Unpin,
{
    writer
        .write_u32(payload.len() as u32)
        .await
        .context("telemetry: failed to write frame length")?;
    writer
        .write_all(payload)
        .await
        .context("telemetry: failed to write frame payload")?;
    writer
        .flush()
        .await
        .context("telemetry: failed to flush frame")?;
    Ok(())
}

/// Incremental decoder whose offsets live outside the returned future.
///
/// `next` can therefore be cancelled by `tokio::select!` after any partial
/// read and safely called again without losing frame alignment.
struct FrameReader<R> {
    inner: R,
    length_bytes: [u8; 4],
    length_read: usize,
    payload: Vec<u8>,
    payload_read: usize,
}

impl<R> FrameReader<R>
where
    R: AsyncRead + Unpin,
{
    fn new(inner: R) -> Self {
        Self {
            inner,
            length_bytes: [0; 4],
            length_read: 0,
            payload: Vec::new(),
            payload_read: 0,
        }
    }

    async fn next<T>(&mut self) -> Result<T>
    where
        T: DeserializeOwned,
    {
        while self.length_read < self.length_bytes.len() {
            let count = self
                .inner
                .read(&mut self.length_bytes[self.length_read..])
                .await
                .context("telemetry: failed to read frame length")?;
            if count == 0 {
                bail!("telemetry: connection closed while reading frame length");
            }
            self.length_read += count;
        }

        if self.payload.is_empty() {
            let length = u32::from_be_bytes(self.length_bytes) as usize;
            if length == 0 || length > MAX_FRAME_SIZE {
                bail!("telemetry: invalid frame length {length}");
            }
            self.payload.resize(length, 0);
        }

        while self.payload_read < self.payload.len() {
            let count = self
                .inner
                .read(&mut self.payload[self.payload_read..])
                .await
                .context("telemetry: failed to read frame payload")?;
            if count == 0 {
                bail!("telemetry: connection closed while reading frame payload");
            }
            self.payload_read += count;
        }

        let payload = std::mem::take(&mut self.payload);
        self.length_bytes = [0; 4];
        self.length_read = 0;
        self.payload_read = 0;
        serde_json::from_slice(&payload).context("telemetry: failed to decode JSON frame")
    }
}

#[cfg(test)]
#[path = "../tests/telemetry/ipc.rs"]
mod tests;
