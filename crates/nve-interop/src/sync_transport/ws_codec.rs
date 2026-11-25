//! Zero-dependency RFC 6455 WebSocket codec (SHA-1 + Base64 + frame codec + HTTP upgrade)
//! and framed binary stream I/O helpers shared by `TcpDeltaServer` and `TcpDeltaClient`.

use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

use super::protocol::{next_nonce, TransportError};

const MAX_WS_UPGRADE_HEADER_BYTES: usize = 8 * 1024;
const WS_GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

fn sha1_digest(data: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [
        0x6745_2301,
        0xEFCD_AB89,
        0x98BA_DCFE,
        0x1032_5476,
        0xC3D2_E1F0,
    ];
    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut padded = Vec::with_capacity(data.len() + 72);
    padded.extend_from_slice(data);
    padded.push(0x80);
    while (padded.len() % 64) != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_len.to_be_bytes());

    let mut w = [0u32; 80];
    for chunk in padded.as_chunks::<64>().0 {
        for (i, &word) in chunk.as_chunks::<4>().0.iter().enumerate() {
            w[i] = u32::from_be_bytes(word);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = h;
        for (i, &wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | ((!b) & d), 0x5A82_7999u32),
                20..=39 => (b ^ c ^ d, 0x6ED9_EBA1u32),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1B_BCDCu32),
                _ => (b ^ c ^ d, 0xCA62_C1D6u32),
            };
            let temp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
    }

    let mut out = [0u8; 20];
    for (i, &val) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&val.to_be_bytes());
    }
    out
}

fn base64_encode(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = u32::from(chunk[0]);
        let b1 = if chunk.len() > 1 { u32::from(chunk[1]) } else { 0 };
        let b2 = if chunk.len() > 2 { u32::from(chunk[2]) } else { 0 };
        let t = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((t >> 18) & 0x3F) as usize] as char);
        out.push(TABLE[((t >> 12) & 0x3F) as usize] as char);
        out.push(if chunk.len() > 1 { TABLE[((t >> 6) & 0x3F) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { TABLE[(t & 0x3F) as usize] as char } else { '=' });
    }
    out
}

/// Computes RFC 6455 `Sec-WebSocket-Accept` from `Sec-WebSocket-Key`.
pub fn compute_ws_accept_key(client_key: &str) -> String {
    let mut concat = String::with_capacity(client_key.len() + WS_GUID.len());
    concat.push_str(client_key.trim());
    concat.push_str(WS_GUID);
    base64_encode(&sha1_digest(concat.as_bytes()))
}

/// RFC 6455 WebSocket frame opcode.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WsOpcode {
    Continuation = 0x0,
    Text = 0x1,
    Binary = 0x2,
    Close = 0x8,
    Ping = 0x9,
    Pong = 0xA,
}

impl WsOpcode {
    pub const fn from_u8(v: u8) -> Result<Self, TransportError> {
        match v {
            0x0 => Ok(Self::Continuation),
            0x1 => Ok(Self::Text),
            0x2 => Ok(Self::Binary),
            0x8 => Ok(Self::Close),
            0x9 => Ok(Self::Ping),
            0xA => Ok(Self::Pong),
            _ => Err(TransportError::WebSocketProtocol("unsupported opcode")),
        }
    }
}

/// Decoded RFC 6455 WebSocket frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WsFrame {
    pub fin: bool,
    pub opcode: WsOpcode,
    pub masked: bool,
    pub payload: Vec<u8>,
}

/// Encodes one RFC 6455 WebSocket frame (`Some(mask)` for client->server, `None` for server->client).
pub fn encode_ws_frame(
    fin: bool,
    opcode: WsOpcode,
    payload: &[u8],
    mask_key: Option<[u8; 4]>,
) -> Vec<u8> {
    let mut out = Vec::with_capacity(14 + payload.len());
    out.push((if fin { 0x80 } else { 0 }) | (opcode as u8));
    let mask_bit = if mask_key.is_some() { 0x80u8 } else { 0u8 };
    let len = payload.len();
    if len <= 125 {
        out.push(mask_bit | (len as u8));
    } else if len <= u16::MAX as usize {
        out.push(mask_bit | 126);
        out.extend_from_slice(&(len as u16).to_be_bytes());
    } else {
        out.push(mask_bit | 127);
        out.extend_from_slice(&(len as u64).to_be_bytes());
    }
    if let Some(mask) = mask_key {
        out.extend_from_slice(&mask);
        for (i, &b) in payload.iter().enumerate() {
            out.push(b ^ mask[i & 3]);
        }
    } else {
        out.extend_from_slice(payload);
    }
    out
}

/// Decodes one RFC 6455 WebSocket frame from a slice, enforcing `max_payload_len` before allocation.
pub fn decode_ws_frame_slice(
    bytes: &[u8],
    max_payload_len: usize,
) -> Result<(WsFrame, usize), TransportError> {
    if bytes.len() < 2 {
        return Err(TransportError::TruncatedFrame);
    }
    let fin = (bytes[0] & 0x80) != 0;
    if (bytes[0] & 0x70) != 0 {
        return Err(TransportError::WebSocketProtocol(
            "non-zero RSV bits without extension",
        ));
    }
    let opcode = WsOpcode::from_u8(bytes[0] & 0x0F)?;
    let masked = (bytes[1] & 0x80) != 0;
    let mut pos = 2usize;

    let payload_len: usize = match bytes[1] & 0x7F {
        n @ 0..=125 => usize::from(n),
        126 => {
            if bytes.len() < pos + 2 {
                return Err(TransportError::TruncatedFrame);
            }
            let n = u16::from_be_bytes(bytes[pos..pos + 2].try_into().unwrap());
            pos += 2;
            usize::from(n)
        }
        127 => {
            if bytes.len() < pos + 8 {
                return Err(TransportError::TruncatedFrame);
            }
            let n_u64 = u64::from_be_bytes(bytes[pos..pos + 8].try_into().unwrap());
            pos += 8;
            if (n_u64 >> 63) != 0 {
                return Err(TransportError::WebSocketProtocol(
                    "MSB set on 64-bit WebSocket payload length",
                ));
            }
            usize::try_from(n_u64).map_err(|_| TransportError::FrameTooLarge {
                declared: usize::MAX,
                max: max_payload_len,
            })?
        }
        _ => unreachable!(),
    };

    if payload_len > max_payload_len {
        return Err(TransportError::FrameTooLarge {
            declared: payload_len,
            max: max_payload_len,
        });
    }
    let mask_key = if masked {
        if bytes.len() < pos + 4 {
            return Err(TransportError::TruncatedFrame);
        }
        let m: [u8; 4] = bytes[pos..pos + 4].try_into().unwrap();
        pos += 4;
        Some(m)
    } else {
        None
    };
    if bytes.len().saturating_sub(pos) < payload_len {
        return Err(TransportError::TruncatedFrame);
    }
    let mut payload = bytes[pos..pos + payload_len].to_vec();
    if let Some(mask) = mask_key {
        for (i, b) in payload.iter_mut().enumerate() {
            *b ^= mask[i & 3];
        }
    }
    pos += payload_len;
    Ok((WsFrame { fin, opcode, masked, payload }, pos))
}

/// Stream framing mode used on a TCP connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamFraming {
    /// 4-byte little-endian length prefix + `NVT1` transport message.
    RawLengthPrefixed,
    /// RFC 6455 HTTP Upgrade + binary WebSocket frames carrying `NVT1` transport messages.
    WebSocketBinary,
}

pub(crate) enum StreamReadOutcome {
    Frame(Vec<u8>),
    Idle,
    Closed,
}

fn read_exact_interruptible(
    stream: &mut TcpStream,
    buf: &mut [u8],
    shutdown: &AtomicBool,
    allow_idle_on_zero: bool,
    stall_timeout: Duration,
) -> Result<bool, TransportError> {
    let mut offset = 0usize;
    let mut started_at: Option<Instant> = None;
    while offset < buf.len() {
        if shutdown.load(Ordering::Relaxed) {
            return Err(TransportError::Disconnected);
        }
        match stream.read(&mut buf[offset..]) {
            Ok(0) => return if offset == 0 { Ok(false) } else { Err(TransportError::TruncatedFrame) },
            Ok(n) => {
                offset += n;
                started_at.get_or_insert_with(Instant::now);
            }
            Err(err)
                if err.kind() == io::ErrorKind::WouldBlock
                    || err.kind() == io::ErrorKind::TimedOut =>
            {
                if offset == 0 && allow_idle_on_zero {
                    return Ok(false);
                }
                let start = *started_at.get_or_insert_with(Instant::now);
                if Instant::now().saturating_duration_since(start) >= stall_timeout {
                    return Err(TransportError::TruncatedFrame);
                }
            }
            Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
            Err(err) => return Err(TransportError::Io(err)),
        }
    }
    Ok(true)
}

pub(crate) fn write_framed_message(
    stream: &mut TcpStream,
    framing: StreamFraming,
    is_client: bool,
    payload: &[u8],
) -> Result<usize, TransportError> {
    match framing {
        StreamFraming::RawLengthPrefixed => {
            let len_u32 = u32::try_from(payload.len()).map_err(|_| {
                TransportError::FrameTooLarge {
                    declared: payload.len(),
                    max: u32::MAX as usize,
                }
            })?;
            let mut framed = Vec::with_capacity(4 + payload.len());
            framed.extend_from_slice(&len_u32.to_le_bytes());
            framed.extend_from_slice(payload);
            stream.write_all(&framed)?;
            Ok(framed.len())
        }
        StreamFraming::WebSocketBinary => {
            let mask = is_client.then(|| (next_nonce(payload.len() as u64) as u32).to_le_bytes());
            let ws_bytes = encode_ws_frame(true, WsOpcode::Binary, payload, mask);
            stream.write_all(&ws_bytes)?;
            Ok(ws_bytes.len())
        }
    }
}

fn read_framed_message(
    stream: &mut TcpStream,
    write_stream: &Mutex<TcpStream>,
    framing: StreamFraming,
    is_client: bool,
    max_frame_bytes: usize,
    shutdown: &AtomicBool,
) -> Result<(StreamReadOutcome, usize), TransportError> {
    let stall_timeout = Duration::from_millis(500);
    match framing {
        StreamFraming::RawLengthPrefixed => {
            let mut len_buf = [0u8; 4];
            if !read_exact_interruptible(stream, &mut len_buf, shutdown, true, stall_timeout)? {
                return Ok((StreamReadOutcome::Idle, 0));
            }
            let declared = u32::from_le_bytes(len_buf) as usize;
            if declared == 0 {
                return Err(TransportError::MalformedFrame("zero-length TCP frame"));
            }
            if declared > max_frame_bytes {
                return Err(TransportError::FrameTooLarge {
                    declared,
                    max: max_frame_bytes,
                });
            }
            let mut payload = vec![0u8; declared];
            if !read_exact_interruptible(stream, &mut payload, shutdown, false, stall_timeout)? {
                return Err(TransportError::TruncatedFrame);
            }
            Ok((StreamReadOutcome::Frame(payload), 4 + declared))
        }
        StreamFraming::WebSocketBinary => {
            let mut assembled: Option<Vec<u8>> = None;
            let mut total_wire = 0usize;
            loop {
                let mut hdr = [0u8; 2];
                if !read_exact_interruptible(
                    stream,
                    &mut hdr,
                    shutdown,
                    assembled.is_none(),
                    stall_timeout,
                )? {
                    return Ok((StreamReadOutcome::Idle, 0));
                }
                total_wire += 2;
                let fin = (hdr[0] & 0x80) != 0;
                if (hdr[0] & 0x70) != 0 {
                    return Err(TransportError::WebSocketProtocol("non-zero RSV bits"));
                }
                let opcode = WsOpcode::from_u8(hdr[0] & 0x0F)?;
                let masked = (hdr[1] & 0x80) != 0;
                let payload_len: usize = match hdr[1] & 0x7F {
                    n @ 0..=125 => usize::from(n),
                    126 => {
                        let mut b = [0u8; 2];
                        read_exact_interruptible(stream, &mut b, shutdown, false, stall_timeout)?;
                        total_wire += 2;
                        usize::from(u16::from_be_bytes(b))
                    }
                    127 => {
                        let mut b = [0u8; 8];
                        read_exact_interruptible(stream, &mut b, shutdown, false, stall_timeout)?;
                        total_wire += 8;
                        let n = u64::from_be_bytes(b);
                        if (n >> 63) != 0 {
                            return Err(TransportError::WebSocketProtocol(
                                "MSB set on 64-bit WebSocket length",
                            ));
                        }
                        usize::try_from(n).map_err(|_| TransportError::FrameTooLarge {
                            declared: usize::MAX,
                            max: max_frame_bytes,
                        })?
                    }
                    _ => unreachable!(),
                };

                let combined = assembled.as_ref().map_or(0, Vec::len).saturating_add(payload_len);
                if payload_len > max_frame_bytes || combined > max_frame_bytes {
                    return Err(TransportError::FrameTooLarge {
                        declared: combined,
                        max: max_frame_bytes,
                    });
                }
                let mask = if masked {
                    let mut m = [0u8; 4];
                    read_exact_interruptible(stream, &mut m, shutdown, false, stall_timeout)?;
                    total_wire += 4;
                    Some(m)
                } else {
                    None
                };
                let mut payload = vec![0u8; payload_len];
                if payload_len > 0 {
                    read_exact_interruptible(stream, &mut payload, shutdown, false, stall_timeout)?;
                    total_wire += payload_len;
                    if let Some(m) = mask {
                        for (i, b) in payload.iter_mut().enumerate() {
                            *b ^= m[i & 3];
                        }
                    }
                }
                match opcode {
                    WsOpcode::Close => return Ok((StreamReadOutcome::Closed, total_wire)),
                    WsOpcode::Ping => {
                        let pm = is_client.then(|| (next_nonce(7) as u32).to_le_bytes());
                        let pong = encode_ws_frame(true, WsOpcode::Pong, &payload, pm);
                        if let Ok(mut w) = write_stream.lock() {
                            let _ = w.write_all(&pong);
                        }
                    }
                    WsOpcode::Pong => {}
                    WsOpcode::Binary | WsOpcode::Text => {
                        if fin && assembled.is_none() {
                            return Ok((StreamReadOutcome::Frame(payload), total_wire));
                        }
                        assembled.get_or_insert_with(Vec::new).extend_from_slice(&payload);
                        if fin {
                            return Ok((StreamReadOutcome::Frame(assembled.unwrap_or_default()), total_wire));
                        }
                    }
                    WsOpcode::Continuation => {
                        let Some(acc) = assembled.as_mut() else {
                            return Err(TransportError::WebSocketProtocol("unexpected continuation frame"));
                        };
                        acc.extend_from_slice(&payload);
                        if fin {
                            return Ok((StreamReadOutcome::Frame(assembled.unwrap_or_default()), total_wire));
                        }
                    }
                }
            }
        }
    }
}

pub(crate) fn read_framed_message_eof_aware(
    stream: &mut TcpStream,
    write_stream: &Mutex<TcpStream>,
    framing: StreamFraming,
    is_client: bool,
    max_frame_bytes: usize,
    shutdown: &AtomicBool,
) -> Result<(StreamReadOutcome, usize), TransportError> {
    let mut peek_one = [0u8; 1];
    match stream.peek(&mut peek_one) {
        Ok(0) => return Ok((StreamReadOutcome::Closed, 0)),
        Ok(_) => {}
        Err(err)
            if err.kind() == io::ErrorKind::WouldBlock
                || err.kind() == io::ErrorKind::TimedOut
                || err.kind() == io::ErrorKind::Interrupted =>
        {
            return Ok((StreamReadOutcome::Idle, 0));
        }
        Err(err) => return Err(TransportError::Io(err)),
    }
    read_framed_message(stream, write_stream, framing, is_client, max_frame_bytes, shutdown)
}

pub(crate) fn detect_stream_framing(
    stream: &TcpStream,
    timeout: Duration,
    shutdown: &AtomicBool,
) -> Result<StreamFraming, TransportError> {
    let deadline = Instant::now() + timeout;
    let mut prefix = [0u8; 4];
    let mut partial_tries = 0u32;
    while Instant::now() < deadline {
        if shutdown.load(Ordering::Relaxed) {
            return Err(TransportError::Disconnected);
        }
        match stream.peek(&mut prefix) {
            Ok(0) => return Err(TransportError::TruncatedFrame),
            Ok(n) if n >= 4 => {
                return Ok(if &prefix[..4] == b"GET " {
                    StreamFraming::WebSocketBinary
                } else {
                    StreamFraming::RawLengthPrefixed
                });
            }
            Ok(n) => {
                if b"GET ".starts_with(&prefix[..n]) && partial_tries < 4 {
                    partial_tries += 1;
                    thread::sleep(Duration::from_millis(4));
                    continue;
                }
                return Ok(StreamFraming::RawLengthPrefixed);
            }
            Err(err)
                if err.kind() == io::ErrorKind::WouldBlock
                    || err.kind() == io::ErrorKind::TimedOut
                    || err.kind() == io::ErrorKind::Interrupted => {}
            Err(err) => return Err(TransportError::Io(err)),
        }
    }
    Err(TransportError::HandshakeTimeout)
}

fn read_http_headers_until_crlfcrlf(
    stream: &mut TcpStream,
    timeout: Duration,
    shutdown: &AtomicBool,
) -> Result<String, TransportError> {
    let deadline = Instant::now() + timeout;
    let mut buf = Vec::with_capacity(512);
    let mut byte = [0u8; 1];
    while Instant::now() < deadline {
        if shutdown.load(Ordering::Relaxed) {
            return Err(TransportError::Disconnected);
        }
        match stream.read(&mut byte) {
            Ok(0) => return Err(TransportError::TruncatedFrame),
            Ok(1) => {
                buf.push(byte[0]);
                if buf.len() > MAX_WS_UPGRADE_HEADER_BYTES {
                    return Err(TransportError::FrameTooLarge {
                        declared: buf.len(),
                        max: MAX_WS_UPGRADE_HEADER_BYTES,
                    });
                }
                if buf.ends_with(b"\r\n\r\n") {
                    return String::from_utf8(buf)
                        .map_err(|_| TransportError::WebSocketProtocol("non-UTF8 HTTP headers"));
                }
            }
            Ok(_) => unreachable!(),
            Err(err)
                if err.kind() == io::ErrorKind::WouldBlock
                    || err.kind() == io::ErrorKind::TimedOut
                    || err.kind() == io::ErrorKind::Interrupted => {}
            Err(err) => return Err(TransportError::Io(err)),
        }
    }
    Err(TransportError::HandshakeTimeout)
}

pub(crate) fn server_handle_ws_upgrade(
    stream: &mut TcpStream,
    timeout: Duration,
    shutdown: &AtomicBool,
) -> Result<(), TransportError> {
    let req = read_http_headers_until_crlfcrlf(stream, timeout, shutdown)?;
    let (mut key, mut has_upgrade) = (None, false);
    for line in req.lines().skip(1) {
        if let Some((name, val)) = line.split_once(':') {
            let nl = name.trim().to_ascii_lowercase();
            let vt = val.trim();
            if nl == "sec-websocket-key" {
                key = Some(vt);
            } else if nl == "upgrade" && vt.eq_ignore_ascii_case("websocket") {
                has_upgrade = true;
            }
        }
    }
    let Some(client_key) = key.filter(|k| !k.is_empty() && has_upgrade) else {
        let _ = stream.write_all(b"HTTP/1.1 400 Bad Request\r\nConnection: close\r\n\r\n");
        return Err(TransportError::WebSocketProtocol(
            "missing Upgrade: websocket or Sec-WebSocket-Key",
        ));
    };
    let accept = compute_ws_accept_key(client_key);
    let resp = format!(
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n"
    );
    stream.write_all(resp.as_bytes())?;
    Ok(())
}

pub(crate) fn client_perform_ws_upgrade(
    stream: &mut TcpStream,
    host: &str,
    path: &str,
    timeout: Duration,
    shutdown: &AtomicBool,
) -> Result<(), TransportError> {
    let mut key_raw = [0u8; 16];
    key_raw[0..8].copy_from_slice(&next_nonce(0xABCD_1234).to_le_bytes());
    key_raw[8..16].copy_from_slice(&next_nonce(0x5678_EF01).to_le_bytes());
    let client_key = base64_encode(&key_raw);
    let expected_accept = compute_ws_accept_key(&client_key);

    let req = format!(
        "GET {path} HTTP/1.1\r\nHost: {host}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {client_key}\r\nSec-WebSocket-Version: 13\r\n\r\n"
    );
    stream.write_all(req.as_bytes())?;
    let resp = read_http_headers_until_crlfcrlf(stream, timeout, shutdown)?;
    if !resp.lines().next().unwrap_or("").contains("101") {
        return Err(TransportError::WebSocketProtocol(
            "server did not respond with HTTP 101 Switching Protocols",
        ));
    }
    let got_accept = resp.lines().skip(1).find_map(|line| {
        let (name, val) = line.split_once(':')?;
        name.trim().eq_ignore_ascii_case("sec-websocket-accept").then(|| val.trim())
    });
    if got_accept != Some(expected_accept.as_str()) {
        return Err(TransportError::WebSocketProtocol("invalid Sec-WebSocket-Accept header"));
    }
    Ok(())
}
