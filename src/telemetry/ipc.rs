// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

//! Protected local IPC transport and per-user registry discovery.

use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use super::local::{self, Listener, Reader, Stream, Writer};
use anyhow::{Context, Result, bail};
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::{Semaphore, broadcast};
use tokio_util::sync::CancellationToken;

use super::process::{process_is_alive, process_uid, read_process_incarnation};
use super::{
    ClientMessage, Hello, MAX_FRAME_SIZE, ServerMessage, Subscription, TELEMETRY_PROTOCOL,
    TelemetryHub,
};

const MAX_CLIENTS: usize = 16;
const WRITE_TIMEOUT: Duration = Duration::from_secs(2);

/// A registry identity validated against the live process incarnation where supported.
#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, Serialize)]
pub(crate) struct DiscoveredInstance {
    pub(crate) registry_name: String,
    pub(crate) uid: u32,
    pub(crate) pid: u32,
    pub(crate) incarnation: u64,
}

#[derive(serde::Deserialize, Serialize)]
struct RegistryEntry {
    instance: DiscoveredInstance,
    endpoint: String,
    transport: String,
    namespace: String,
    protocol: String,
}

/// Publishes one process hub to any number of read-only TUI clients.
pub(crate) struct TelemetryServer {
    listener: Listener,
    endpoint: String,
    hub: Arc<TelemetryHub>,
    clients: Arc<Semaphore>,
    registry_path: PathBuf,
}

impl TelemetryServer {
    pub(crate) fn bind(hub: Arc<TelemetryHub>) -> Result<Self> {
        if let Some(reason) = hub.unavailable_reason() {
            bail!("telemetry process identity is unavailable: {reason}");
        }
        let descriptor = hub.descriptor();
        let name = descriptor.registry_name();
        local::prepare_directory()?;
        let endpoint = local::endpoint(&descriptor.id);
        let listener = Listener::bind(&endpoint)?;
        let entry = RegistryEntry {
            instance: DiscoveredInstance {
                registry_name: name,
                uid: descriptor.uid,
                pid: descriptor.pid,
                incarnation: descriptor.incarnation,
            },
            endpoint: endpoint.clone(),
            transport: local::TRANSPORT.to_owned(),
            namespace: local::namespace(),
            protocol: TELEMETRY_PROTOCOL.to_owned(),
        };
        let registry_path = registry_path(&entry.instance.registry_name);
        let payload = match serde_json::to_vec(&entry) {
            Ok(value) => value,
            Err(error) => {
                local::cleanup(&endpoint);
                return Err(error.into());
            }
        };
        if let Err(error) = local::publish(&registry_path, &payload) {
            local::cleanup(&endpoint);
            return Err(error);
        }
        Ok(Self {
            listener,
            endpoint,
            hub,
            clients: Arc::new(Semaphore::new(MAX_CLIENTS)),
            registry_path,
        })
    }

    pub(crate) async fn run(mut self, shutdown: CancellationToken) {
        let mut tasks = tokio::task::JoinSet::new();
        loop {
            let accepted = tokio::select! {
                Some(_) = tasks.join_next(), if !tasks.is_empty() => continue,
                _ = shutdown.cancelled() => break,
                accepted = self.listener.accept() => accepted,
            };
            let Ok(stream) = accepted else {
                tokio::select! {
                    _ = shutdown.cancelled() => break,
                    _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => {}
                }
                continue;
            };
            let Ok(permit) = Arc::clone(&self.clients).try_acquire_owned() else {
                drop(stream);
                continue;
            };
            let hub = Arc::clone(&self.hub);
            let connection_shutdown = shutdown.clone();
            tasks.spawn(async move {
                let _permit = permit;
                let _ = serve_client(stream, hub, connection_shutdown).await;
            });
        }
        tasks.abort_all();
        while tasks.join_next().await.is_some() {}
    }
}

async fn serve_client(
    stream: Stream,
    hub: Arc<TelemetryHub>,
    shutdown: CancellationToken,
) -> Result<()> {
    let (reader, mut writer) = tokio::io::split(stream);
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
    let mut events;
    let command = tokio::select! {
        _ = shutdown.cancelled() => return Ok(()),
        result = tokio::time::timeout(Duration::from_secs(5), reader.next_command()) => result??,
    };
    let ClientMessage::Subscribe {
        request_id,
        mut subscription,
    } = command;
    events = hub.event_receiver();
    let mut _detail = (subscription == Subscription::Detail).then(|| hub.detail_guard());
    write_frame(
        &mut writer,
        &ServerMessage::Subscribed {
            request_id,
            subscription,
        },
    )
    .await?;
    let initial_snapshot = snapshots.borrow_and_update().clone();
    write_frame(&mut writer, &ServerMessage::Snapshot(initial_snapshot)).await?;
    let initial_lifecycle = lifecycles.borrow_and_update().clone();
    write_frame(&mut writer, &ServerMessage::Lifecycle(initial_lifecycle)).await?;
    let mut tokens = 7.0f64;
    let mut command_at = std::time::Instant::now();
    loop {
        tokio::select! {
            _ = shutdown.cancelled() => return Ok(()),
            command = reader.next_command() => {
                match command? {
                    ClientMessage::Subscribe { request_id, subscription: next } => {
                        let now = std::time::Instant::now();
                        tokens = (tokens + now.duration_since(command_at).as_secs_f64() * 4.0).min(8.0);
                        command_at = now;
                        if tokens < 1.0 { bail!("command rate exceeded"); }
                        tokens -= 1.0;
                        if next != subscription {
                            events = hub.event_receiver();
                            _detail = (next == Subscription::Detail).then(|| hub.detail_guard());
                        }
                        subscription = next;
                        write_frame(&mut writer, &ServerMessage::Subscribed { request_id, subscription }).await?;
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
            event = events.recv(), if subscription == Subscription::Detail => {
                match event {
                    Ok(event) => {
                        write_frame(&mut writer, &event).await?;
                    }
                    Err(broadcast::error::RecvError::Lagged(missed)) => {
                        write_frame(
                            &mut writer,
                            &ServerMessage::Gap { missed },
                        ).await?;
                    }
                    Err(broadcast::error::RecvError::Closed) => return Ok(()),
                }
            }
        }
    }
}

pub(crate) fn discover_instances() -> io::Result<Vec<DiscoveredInstance>> {
    let current_uid = process_uid();
    if !registry_directory().exists() {
        return Ok(Vec::new());
    }
    local::validate_directory(&registry_directory()).map_err(io::Error::other)?;
    let mut found = Vec::new();
    let directory = registry_directory();
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(found),
        Err(error) => return Err(error),
    };
    for path in entries.filter_map(Result::ok).map(|entry| entry.path()) {
        let Ok(payload) = local::read_registry(&path) else {
            continue;
        };
        let Ok(entry) = serde_json::from_slice::<RegistryEntry>(&payload) else {
            continue;
        };
        let instance = entry.instance;
        if entry.transport != local::TRANSPORT
            || entry.protocol != TELEMETRY_PROTOCOL
            || entry.namespace != local::namespace()
            || !valid_instance(&instance)
            || path != registry_path(&instance.registry_name)
            || entry.endpoint
                != local::endpoint(
                    instance
                        .registry_name
                        .strip_prefix("nowhere.")
                        .unwrap_or_default(),
                )
        {
            continue;
        }
        if instance.uid != current_uid {
            continue;
        }
        if !process_is_alive(instance.pid)
            || ((cfg!(target_os = "linux") || cfg!(windows))
                && read_process_incarnation(instance.pid)
                    .is_some_and(|incarnation| incarnation != instance.incarnation))
        {
            local::cleanup(&entry.endpoint);
            let _ = std::fs::remove_file(path);
            continue;
        }
        found.push(instance);
    }
    found.sort_by_key(|instance| (instance.uid, instance.pid, instance.incarnation));
    found.dedup();
    Ok(found)
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
        if !valid_instance(discovered) {
            bail!("invalid registry name");
        }
        let payload = local::read_registry(&registry_path(&discovered.registry_name))
            .context("telemetry: discovered registry disappeared")?;
        let entry: RegistryEntry =
            serde_json::from_slice(&payload).context("telemetry: invalid discovered registry")?;
        if entry.instance != *discovered
            || entry.transport != local::TRANSPORT
            || entry.protocol != TELEMETRY_PROTOCOL
            || entry.namespace != local::namespace()
            || !valid_instance(discovered)
            || entry.endpoint
                != local::endpoint(
                    discovered
                        .registry_name
                        .strip_prefix("nowhere.")
                        .expect("valid name"),
                )
        {
            bail!("telemetry: discovered registry identity mismatch");
        }
        let stream = tokio::time::timeout(
            Duration::from_secs(5),
            local::connect(&entry.endpoint, discovered.pid),
        )
        .await??;
        let (reader, mut writer) = tokio::io::split(stream);
        let mut reader = FrameReader::new(reader);
        let message = tokio::time::timeout(Duration::from_secs(5), reader.next::<ServerMessage>())
            .await
            .context("telemetry: hello timed out")??;
        let mut hello = match message {
            ServerMessage::Hello(hello) => hello,
            ServerMessage::Error { message } => {
                bail!("telemetry: service rejected connection: {message}")
            }
            _ => bail!("telemetry: service did not begin with hello"),
        };
        validate_hello(&hello, discovered)?;
        // These local identity fields are intentionally absent from wire JSON.
        hello.instance.uid = discovered.uid;
        hello.instance.incarnation = discovered.incarnation;
        write_frame(
            &mut writer,
            &ClientMessage::Subscribe {
                request_id: 0,
                subscription,
            },
        )
        .await?;
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

impl Drop for TelemetryServer {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.registry_path);
        local::cleanup(&self.endpoint);
    }
}

fn registry_directory() -> PathBuf {
    local::directory()
}
fn valid_instance(instance: &DiscoveredInstance) -> bool {
    instance.pid > 0
        && (!cfg!(unix) || instance.pid <= i32::MAX as u32)
        && instance
            .registry_name
            .strip_prefix("nowhere.")
            .is_some_and(|id| {
                id.len() == 32
                    && id
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            })
}

fn registry_path(registry_name: &str) -> PathBuf {
    registry_directory().join(format!("{registry_name}.json"))
}

fn validate_hello(hello: &Hello, discovered: &DiscoveredInstance) -> Result<()> {
    let instance = &hello.instance;
    if instance.telemetry_protocol != TELEMETRY_PROTOCOL
        || instance.pid != discovered.pid
        || instance.registry_name() != discovered.registry_name
    {
        bail!("telemetry: hello identity does not match discovered registry");
    }
    Ok(())
}

pub(crate) struct TelemetryReader {
    inner: FrameReader<Reader>,
}

impl TelemetryReader {
    pub(crate) async fn next_message(&mut self) -> Result<ServerMessage> {
        self.inner.next().await
    }
}

pub(crate) struct TelemetryWriter {
    inner: Writer,
}

impl TelemetryWriter {
    pub(crate) async fn subscribe(&mut self, subscription: Subscription) -> Result<()> {
        write_frame(
            &mut self.inner,
            &ClientMessage::Subscribe {
                request_id: 0,
                subscription,
            },
        )
        .await
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
    started_at: Option<tokio::time::Instant>,
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
            started_at: None,
        }
    }

    fn check_deadline(&self) -> Result<()> {
        if self
            .started_at
            .is_some_and(|start| start.elapsed() >= Duration::from_secs(5))
        {
            bail!("partial frame timeout");
        }
        Ok(())
    }

    async fn next_command(&mut self) -> Result<ClientMessage> {
        self.read_frame(1024).await
    }
    async fn next<T: DeserializeOwned>(&mut self) -> Result<T> {
        self.read_frame(MAX_FRAME_SIZE).await
    }
    async fn read_frame<T>(&mut self, limit: usize) -> Result<T>
    where
        T: DeserializeOwned,
    {
        while self.length_read < self.length_bytes.len() {
            self.check_deadline()?;
            let read = self.inner.read(&mut self.length_bytes[self.length_read..]);
            let count = if let Some(start) = self.started_at {
                tokio::time::timeout_at(start + Duration::from_secs(5), read)
                    .await
                    .context("partial frame timeout")?
            } else {
                read.await
            }
            .context("telemetry: failed to read frame length")?;
            if count == 0 {
                bail!("telemetry: connection closed while reading frame length");
            }
            self.started_at
                .get_or_insert_with(tokio::time::Instant::now);
            self.length_read += count;
        }

        if self.payload.is_empty() {
            let length = u32::from_be_bytes(self.length_bytes) as usize;
            if length == 0 || length > limit {
                bail!("telemetry: invalid frame length {length}");
            }
            self.payload.resize(length, 0);
        }

        while self.payload_read < self.payload.len() {
            self.check_deadline()?;
            let count = tokio::time::timeout_at(
                self.started_at.expect("frame started") + Duration::from_secs(5),
                self.inner.read(&mut self.payload[self.payload_read..]),
            )
            .await
            .context("partial frame timeout")?
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
        self.started_at = None;
        serde_json::from_slice(&payload).context("telemetry: failed to decode JSON frame")
    }
}

#[cfg(test)]
#[path = "../tests/telemetry/ipc.rs"]
mod tests;
