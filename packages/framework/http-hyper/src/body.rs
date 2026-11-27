//! Streaming bounded HTTP body reader with zero-allocation single-frame fast path and deadlines.

use core::time::Duration;

use bytes::{Bytes, BytesMut};
use http_body_util::BodyExt as _;
use hyper::body::Body;

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
