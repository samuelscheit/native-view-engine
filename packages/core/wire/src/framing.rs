//! Unified async stream helpers for reading & writing `NveWireFrame` v2 messages on QUIC
//! streams, HTTP/3 WebTransport streams, and WebSocket binary messages with pre-allocation bounds.

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use nve_wire_core::{
    NveWireFrame, NveWireHeader, NVE_WIRE_HEADER_SIZE, NVE_WIRE_TRAILER_SIZE,
};

use crate::NveWireTransportError;

/// Writes one `NveWireFrame` to an async stream (`quinn::SendStream` or `web_transport_quinn::SendStream`).
pub async fn write_stream_frame<W: AsyncWrite + Unpin>(
    writer: &mut W,
    frame: &NveWireFrame,
) -> Result<usize, NveWireTransportError> {
    let encoded = frame.encode()?;
    writer
        .write_all(&encoded)
        .await
        .map_err(|e| NveWireTransportError::Io(e.to_string()))?;
    Ok(encoded.len())
}

/// Writes pre-encoded `NveWireFrame` bytes to an async stream.
pub async fn write_stream_raw<W: AsyncWrite + Unpin>(
    writer: &mut W,
    encoded: &[u8],
) -> Result<(), NveWireTransportError> {
    writer
        .write_all(encoded)
        .await
        .map_err(|e| NveWireTransportError::Io(e.to_string()))
}

/// Reads one `NveWireFrame` from an async stream, enforcing `max_frame_bytes` BEFORE allocating
/// the payload buffer. Returns `Ok(None)` on clean EOF.
pub async fn read_stream_frame<R: AsyncRead + Unpin>(
    reader: &mut R,
    max_frame_bytes: usize,
) -> Result<Option<NveWireFrame>, NveWireTransportError> {
    let mut header_bytes = [0u8; NVE_WIRE_HEADER_SIZE];
    match reader.read_exact(&mut header_bytes).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(NveWireTransportError::Io(e.to_string())),
    }

    let header = NveWireHeader::decode(&header_bytes, max_frame_bytes)?;
    let trailer_len = if header.has_xxh64() {
        NVE_WIRE_TRAILER_SIZE
    } else {
        0
    };
    let total_len = NVE_WIRE_HEADER_SIZE + header.payload_len as usize + trailer_len;
    let mut full = vec![0u8; total_len];
    full[..NVE_WIRE_HEADER_SIZE].copy_from_slice(&header_bytes);
    if total_len > NVE_WIRE_HEADER_SIZE {
        reader
            .read_exact(&mut full[NVE_WIRE_HEADER_SIZE..])
            .await
            .map_err(|e| NveWireTransportError::Io(e.to_string()))?;
    }

    let frame = NveWireFrame::decode(&full, max_frame_bytes)?;
    Ok(Some(frame))
}

/// Reads one `NveWireFrame` from a `web_transport_quinn::RecvStream`.
pub async fn read_wt_frame(
    recv: &mut web_transport_quinn::RecvStream,
    max_frame_bytes: usize,
) -> Result<Option<NveWireFrame>, NveWireTransportError> {
    let mut header_bytes = [0u8; NVE_WIRE_HEADER_SIZE];
    if !read_wt_exact(recv, &mut header_bytes).await? {
        return Ok(None);
    }
    let header = NveWireHeader::decode(&header_bytes, max_frame_bytes)?;
    let trailer_len = if header.has_xxh64() {
        NVE_WIRE_TRAILER_SIZE
    } else {
        0
    };
    let total_len = NVE_WIRE_HEADER_SIZE + header.payload_len as usize + trailer_len;
    let mut full = vec![0u8; total_len];
    full[..NVE_WIRE_HEADER_SIZE].copy_from_slice(&header_bytes);
    if total_len > NVE_WIRE_HEADER_SIZE && !read_wt_exact(recv, &mut full[NVE_WIRE_HEADER_SIZE..]).await? {
        return Err(NveWireTransportError::Io(
            "unexpected EOF while reading WebTransport frame body".to_owned(),
        ));
    }
    Ok(Some(NveWireFrame::decode(&full, max_frame_bytes)?))
}

async fn read_wt_exact(
    recv: &mut web_transport_quinn::RecvStream,
    buf: &mut [u8],
) -> Result<bool, NveWireTransportError> {
    let mut filled = 0usize;
    while filled < buf.len() {
        match recv.read(&mut buf[filled..]).await {
            Ok(Some(0)) | Ok(None) => {
                if filled == 0 {
                    return Ok(false);
                }
                return Err(NveWireTransportError::Io(
                    "truncated WebTransport stream".to_owned(),
                ));
            }
            Ok(Some(n)) => filled += n,
            Err(e) => return Err(NveWireTransportError::Io(e.to_string())),
        }
    }
    Ok(true)
}

/// Writes pre-encoded `NveWireFrame` bytes to a `web_transport_quinn::SendStream`.
pub async fn write_wt_raw(
    send: &mut web_transport_quinn::SendStream,
    encoded: &[u8],
) -> Result<(), NveWireTransportError> {
    send.write_all(encoded)
        .await
        .map_err(|e| NveWireTransportError::Io(e.to_string()))
}
