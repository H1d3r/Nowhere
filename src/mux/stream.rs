// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

use std::io;
use std::io::IoSlice;
use std::pin::Pin;
use std::task::{Context, Poll};

use super::wire::FLAG_FIN;
use bytes::Bytes;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::sync::oneshot;

use super::driver::{closed, frame_stream, send_data};
use super::{FRAME_BYTES, FlowReader, FlowWriter, Inbound, MuxStream, Outbound};

impl MuxStream {
    pub fn into_split(self) -> (FlowReader, FlowWriter) {
        (self.reader, self.writer)
    }

    pub fn flow_id(&self) -> u32 {
        self.writer.flow_id
    }
}

impl AsyncRead for MuxStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.reader).poll_read(cx, buf)
    }
}

impl AsyncWrite for MuxStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.writer).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.writer).poll_flush(cx)
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.writer).poll_write_vectored(cx, bufs)
    }

    fn is_write_vectored(&self) -> bool {
        true
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.writer).poll_shutdown(cx)
    }
}

impl AsyncRead for FlowReader {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        loop {
            if let Some((payload, offset, charge)) = self.current.take() {
                let count = (payload.len() - offset).min(buf.remaining());
                buf.put_slice(&payload[offset..offset + count]);
                let next = offset + count;
                if next == payload.len() {
                    self.shared.release_receive(self.flow_id, charge);
                } else {
                    self.current = Some((payload, next, charge));
                }
                return Poll::Ready(Ok(()));
            }
            if self.eof {
                return Poll::Ready(Ok(()));
            }
            match Pin::new(&mut self.receiver).poll_recv(cx) {
                Poll::Ready(Some(Inbound::Data { payload, charge })) => {
                    self.current = Some((payload, 0, charge));
                }
                Poll::Ready(Some(Inbound::Fin)) | Poll::Ready(None) => {
                    self.eof = true;
                    return Poll::Ready(Ok(()));
                }
                Poll::Ready(Some(Inbound::Reset)) => {
                    self.eof = true;
                    return Poll::Ready(Err(io::Error::new(
                        io::ErrorKind::ConnectionReset,
                        "mux flow reset",
                    )));
                }
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

impl Drop for FlowReader {
    fn drop(&mut self) {
        if let Some((_, _, charge)) = self.current.take() {
            self.shared.release_receive(self.flow_id, charge);
        }
        while let Ok(inbound) = self.receiver.try_recv() {
            if let Inbound::Data { charge, .. } = inbound {
                self.shared.release_receive(self.flow_id, charge);
            }
        }
        self.shared.release_part(self.flow_id);
    }
}

impl AsyncWrite for FlowWriter {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        if self.closed {
            return Poll::Ready(Err(closed()));
        }
        if let Some(result) = self.poll_pending(cx) {
            return result;
        }
        if buf.is_empty() {
            return Poll::Ready(Ok(0));
        }
        let length = buf.len().min(FRAME_BYTES);
        let payload = Bytes::copy_from_slice(&buf[..length]);
        self.start_write(cx, payload, length)
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        if self.closed {
            return Poll::Ready(Err(closed()));
        }
        if let Some(result) = self.poll_pending(cx) {
            return result;
        }
        let length = bufs
            .iter()
            .map(|buffer| buffer.len())
            .sum::<usize>()
            .min(FRAME_BYTES);
        if length == 0 {
            return Poll::Ready(Ok(0));
        }
        let mut payload = Vec::with_capacity(length);
        for buffer in bufs {
            let remaining = length - payload.len();
            if remaining == 0 {
                break;
            }
            payload.extend_from_slice(&buffer[..buffer.len().min(remaining)]);
        }
        self.start_write(cx, Bytes::from(payload), length)
    }

    fn is_write_vectored(&self) -> bool {
        true
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.poll_pending(cx) {
            Some(Poll::Pending) => return Poll::Pending,
            Some(Poll::Ready(Err(error))) => return Poll::Ready(Err(error)),
            _ => {}
        }
        if self.pending_action.is_none() {
            let shared = self.shared.clone();
            let flow_id = self.flow_id;
            self.pending_action = Some(Box::pin(async move {
                let (tx, rx) = oneshot::channel();
                shared
                    .data_tx
                    .send(Outbound {
                        header: frame_stream(flow_id, 0, 0)?,
                        payload: Bytes::new(),
                        flushed: Some(tx),
                    })
                    .await
                    .map_err(|_| closed())?;
                rx.await.map_err(|_| closed())?
            }));
        }
        self.poll_action(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if self.closed {
            return Poll::Ready(Ok(()));
        }
        match self.poll_pending(cx) {
            Some(Poll::Pending) => return Poll::Pending,
            Some(Poll::Ready(Err(error))) => return Poll::Ready(Err(error)),
            _ => {}
        }
        if self.pending_action.is_none() {
            let shared = self.shared.clone();
            let flow_id = self.flow_id;
            self.pending_action = Some(Box::pin(async move {
                shared
                    .data_tx
                    .send(Outbound {
                        header: frame_stream(flow_id, FLAG_FIN, 0)?,
                        payload: Bytes::new(),
                        flushed: None,
                    })
                    .await
                    .map_err(|_| closed())
            }));
        }
        match self.poll_action(cx) {
            Poll::Ready(Ok(())) => {
                self.closed = true;
                self.terminal_permit = None;
                Poll::Ready(Ok(()))
            }
            other => other,
        }
    }
}

impl FlowWriter {
    fn start_write(
        &mut self,
        cx: &mut Context<'_>,
        payload: Bytes,
        length: usize,
    ) -> Poll<io::Result<usize>> {
        let shared = self.shared.clone();
        let flow_id = self.flow_id;
        self.pending = Some(Box::pin(async move {
            send_data(shared, flow_id, payload).await?;
            Ok(length)
        }));
        self.poll_pending(cx).expect("mux write future installed")
    }
    fn poll_pending(&mut self, cx: &mut Context<'_>) -> Option<Poll<io::Result<usize>>> {
        let result = self.pending.as_mut()?.as_mut().poll(cx);
        if result.is_ready() {
            self.pending = None;
        }
        Some(result)
    }

    fn poll_action(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let result = self
            .pending_action
            .as_mut()
            .expect("mux action future installed")
            .as_mut()
            .poll(cx);
        if result.is_ready() {
            self.pending_action = None;
        }
        result
    }
}

impl Drop for FlowWriter {
    fn drop(&mut self) {
        if !self.closed {
            // One bounded dispatcher per carrier preserves ordering behind
            // already queued DATA without spawning a task for every dropped
            // stream. Dropping a writer is a half-close: split-direction users
            // intentionally discard the unused half while retaining the other.
            if let Some(permit) = self.terminal_permit.take() {
                permit.send(self.flow_id);
            }
        }
        self.shared.release_part(self.flow_id);
    }
}
