//! Streaming bounded HTTP body reader (zero-allocation single-frame fast path with deadlines)
//! and zero-dependency streaming response body primitives ([`ChannelBody`], [`HttpBody`]).

use core::convert::Infallible;
use core::pin::Pin;
use core::task::{Context, Poll};
use core::time::Duration;

use bytes::{Bytes, BytesMut};
use http_body_util::{BodyExt as _, Full};
use hyper::body::{Body, Frame, SizeHint};
use tokio::sync::mpsc;

/// Streaming HTTP response body backed by a bounded [`tokio::sync::mpsc::Receiver<Bytes>`].
///
/// Dropping the body (e.g., when a client closes its TCP connection mid-stream) drops the
/// receiver, closing the channel so the producer task's `tx.closed()` / `tx.send(...)` wakes and
/// terminates promptly without leaking background work.
#[derive(Debug)]
pub struct ChannelBody {
    rx: mpsc::Receiver<Bytes>,
    ended: bool,
}

impl ChannelBody {
    /// Wraps a bounded `tokio::sync::mpsc::Receiver<Bytes>` as a streaming [`Body`].
    #[inline]
    pub fn new(rx: mpsc::Receiver<Bytes>) -> Self {
        Self { rx, ended: false }
    }
}

impl Body for ChannelBody {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        if self.ended {
            return Poll::Ready(None);
        }
        loop {
            match self.rx.poll_recv(cx) {
                Poll::Ready(Some(chunk)) => {
                    if chunk.is_empty() {
                        continue;
                    }
                    return Poll::Ready(Some(Ok(Frame::data(chunk))));
                }
                Poll::Ready(None) => {
                    self.ended = true;
                    return Poll::Ready(None);
                }
                Poll::Pending => return Poll::Pending,
            }
        }
    }

    #[inline]
    fn is_end_stream(&self) -> bool {
        self.ended
    }
}

/// Unified HTTP response body supporting both zero-overhead buffered [`Full<Bytes>`] responses
/// (preserving exact `Content-Length`) and backpressured streaming [`ChannelBody`] responses
/// (allowing Hyper to drive HTTP/1.1 chunked framing or HTTP/2 `DATA` frames automatically).
#[derive(Debug)]
pub enum HttpBody {
    Full(Full<Bytes>),
    Stream(ChannelBody),
}

impl Default for HttpBody {
    #[inline]
    fn default() -> Self {
        Self::Full(Full::new(Bytes::new()))
    }
}

impl HttpBody {
    /// Constructs a buffered [`HttpBody::Full`] response body with zero extra allocation.
    #[inline]
    pub fn full(bytes: Bytes) -> Self {
        Self::Full(Full::new(bytes))
    }

    /// Constructs a streaming [`HttpBody::Stream`] backed by a bounded `mpsc::Receiver<Bytes>`.
    #[inline]
    pub fn stream(rx: mpsc::Receiver<Bytes>) -> Self {
        Self::Stream(ChannelBody::new(rx))
    }
}

impl From<Full<Bytes>> for HttpBody {
    #[inline]
    fn from(full: Full<Bytes>) -> Self {
        Self::Full(full)
    }
}

impl From<Bytes> for HttpBody {
    #[inline]
    fn from(bytes: Bytes) -> Self {
        Self::full(bytes)
    }
}

impl From<ChannelBody> for HttpBody {
    #[inline]
    fn from(stream: ChannelBody) -> Self {
        Self::Stream(stream)
    }
}

impl Body for HttpBody {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        match self.get_mut() {
            Self::Full(full) => Pin::new(full).poll_frame(cx),
            Self::Stream(stream) => Pin::new(stream).poll_frame(cx),
        }
    }

    #[inline]
    fn is_end_stream(&self) -> bool {
        match self {
            Self::Full(full) => full.is_end_stream(),
            Self::Stream(stream) => stream.is_end_stream(),
        }
    }

    #[inline]
    fn size_hint(&self) -> SizeHint {
        match self {
            Self::Full(full) => full.size_hint(),
            Self::Stream(stream) => stream.size_hint(),
        }
    }
}

/// Failure returned when reading an HTTP stream exceeds byte limits, times out, or errors out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BodyReadError {
    TooLarge { limit: usize },
    Timeout { timeout: Duration },
    Stream(String),
}

impl core::fmt::Display for BodyReadError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::TooLarge { limit } => write!(f, "HTTP body exceeded limit of {limit} bytes"),
            Self::Timeout { timeout } => {
                write!(f, "HTTP body read timed out after {}ms", timeout.as_millis())
            }
            Self::Stream(msg) => write!(f, "HTTP body stream error: {msg}"),
        }
    }
}

impl std::error::Error for BodyReadError {}

/// Reads an HTTP body stream to completion (`None`) while enforcing `max_bytes` incrementally per
/// frame so oversized payloads abort immediately without allocating unbounded memory.
///
/// Single-frame bodies return the frame's [`Bytes`] directly with zero extra heap allocation.
pub async fn collect_body_limited<B>(mut body: B, max_bytes: usize) -> Result<Bytes, BodyReadError>
where
    B: Body<Data = Bytes> + Unpin,
    B::Error: core::fmt::Display,
{
    let hint = body.size_hint();
    if hint.lower() > max_bytes as u64 {
        return Err(BodyReadError::TooLarge { limit: max_bytes });
    }

    let mut first: Option<Bytes> = None;
    let mut buf: Option<BytesMut> = None;
    let mut total_len = 0usize;

    while let Some(frame_res) = body.frame().await {
        let frame = frame_res.map_err(|err| BodyReadError::Stream(err.to_string()))?;
        if let Ok(data) = frame.into_data() {
            if data.is_empty() {
                continue;
            }
            total_len = total_len
                .checked_add(data.len())
                .ok_or(BodyReadError::TooLarge { limit: max_bytes })?;
            if total_len > max_bytes {
                return Err(BodyReadError::TooLarge { limit: max_bytes });
            }
            if let Some(acc) = &mut buf {
                acc.extend_from_slice(&data);
            } else if let Some(prev) = first.take() {
                let mut acc = BytesMut::with_capacity(total_len);
                acc.extend_from_slice(&prev);
                acc.extend_from_slice(&data);
                buf = Some(acc);
            } else {
                first = Some(data);
            }
        }
    }

    Ok(match (buf, first) {
        (Some(acc), _) => acc.freeze(),
        (None, Some(single)) => single,
        (None, None) => Bytes::new(),
    })
}

/// Runs [`collect_body_limited`] bounded by `timeout`, returning [`BodyReadError::Timeout`] if a
/// slow client stalls mid-body.
pub async fn collect_body_limited_timeout<B>(
    body: B,
    max_bytes: usize,
    timeout: Duration,
) -> Result<Bytes, BodyReadError>
where
    B: Body<Data = Bytes> + Unpin,
    B::Error: core::fmt::Display,
{
    match tokio::time::timeout(timeout, collect_body_limited(body, max_bytes)).await {
        Ok(res) => res,
        Err(_) => Err(BodyReadError::Timeout { timeout }),
    }
}
