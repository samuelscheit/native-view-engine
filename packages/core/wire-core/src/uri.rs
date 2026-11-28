//! Self-documenting pairing URI parser & formatter for QUIC (`nve-wire-2`), WebTransport (`h3`),
//! and TLS WebSocket (`wss://`) endpoints with pinned SHA-256 certificate hashes.

use crate::frame::NveWireError;

/// Default QUIC ALPN identifier for native `nve-wire` peers.
pub const DEFAULT_WIRE_QUIC_ALPN: &str = "nve-wire-2";

/// Default QUIC / WebTransport UDP port for `nve pair`.
pub const DEFAULT_WIRE_QUIC_PORT: u16 = 8091;

/// Default TLS WebSocket (`wss://`) fallback port for `nve pair`.
pub const DEFAULT_WIRE_WSS_PORT: u16 = 8092;

/// Default WebSocket / WebTransport path.
pub const DEFAULT_WIRE_WSS_PATH: &str = "/nve-wire";

/// Transport selection mode for `nve-wire` clients.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, nve_codec::ReprEnum)]
#[repr_enum(fallback = Auto)]
pub enum NveWireTransportKind {
    /// Race QUIC first, fall back to WSS after 250ms (Happy Eyeballs).
    Auto = 0,
    /// Native raw QUIC (`nve-wire-2` ALPN).
    Quic = 1,
    /// HTTP/3 WebTransport (`h3` ALPN).
    WebTransport = 2,
    /// TLS WebSocket (`wss://`).
    Wss = 3,
    /// Localhost plain WebSocket (`ws://`, loopback dev only).
    Ws = 4,
}

impl NveWireTransportKind {
    pub fn parse(raw: &str) -> Result<Self, NveWireError> {
        let s = raw.trim();
        if s.is_empty()
            || s.eq_ignore_ascii_case("auto")
            || s.eq_ignore_ascii_case("happy-eyeballs")
        {
            Ok(Self::Auto)
        } else if s.eq_ignore_ascii_case("quic") || s.eq_ignore_ascii_case("udp") {
            Ok(Self::Quic)
        } else if s.eq_ignore_ascii_case("wt")
            || s.eq_ignore_ascii_case("webtransport")
            || s.eq_ignore_ascii_case("h3")
        {
            Ok(Self::WebTransport)
        } else if s.eq_ignore_ascii_case("wss") || s.eq_ignore_ascii_case("tls") {
            Ok(Self::Wss)
        } else if s.eq_ignore_ascii_case("ws")
            || s.eq_ignore_ascii_case("tcp")
            || s.eq_ignore_ascii_case("websocket")
        {
            Ok(Self::Ws)
        } else {
            Err(NveWireError::InvalidPayload("unknown transport kind"))
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Quic => "quic",
            Self::WebTransport => "webtransport",
            Self::Wss => "wss",
            Self::Ws => "ws",
        }
    }
}

/// Parsed pairing / wire endpoint URI (`nve://pair?...`, `quic://`, `https://`, `wss://`, `ws://`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NveWireUri {
    pub host: String,
    pub quic_port: Option<u16>,
    pub wss_port: Option<u16>,
    pub path: String,
    pub transport: NveWireTransportKind,
    pub token: String,
    pub cert_sha256: Option<[u8; 32]>,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

/// Formats a 32-byte SHA-256 certificate digest as 64 lowercase hex characters.
pub fn format_cert_hash_hex(hash: &[u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(64);
    for &b in hash {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0F) as usize] as char);
    }
    out
}

/// Parses a 64-character hex string (with optional `:` separators) into a 32-byte SHA-256 hash
/// with zero heap allocations.
pub fn parse_cert_hash_hex(hex: &str) -> Result<[u8; 32], NveWireError> {
    let mut cleaned = [0u8; 64];
    let mut count = 0usize;
    for b in hex.bytes() {
        if b == b':' || b.is_ascii_whitespace() {
            continue;
        }
        if count == 64 {
            return Err(NveWireError::InvalidPayload("cert_hash must be 64 hex chars"));
        }
        cleaned[count] = b;
        count += 1;
    }
    if count != 64 {
        return Err(NveWireError::InvalidPayload("cert_hash must be 64 hex chars"));
    }
    let mut out = [0u8; 32];
    for i in 0..32 {
        let hi = decode_hex_nibble(cleaned[i * 2])?;
        let lo = decode_hex_nibble(cleaned[i * 2 + 1])?;
        out[i] = (hi << 4) | lo;
    }
    Ok(out)
}

#[inline]
fn decode_hex_nibble(b: u8) -> Result<u8, NveWireError> {
    match b {
        b'0'..=b'9' => Ok(b - b'0'),
        b'a'..=b'f' => Ok(b - b'a' + 10),
        b'A'..=b'F' => Ok(b - b'A' + 10),
        _ => Err(NveWireError::InvalidPayload("invalid hex character in cert_hash")),
    }
}

/// Formats a canonical `nve://pair?...` URI containing QUIC port, WSS port, Bearer token, and cert hash.
pub fn format_wire_uri(
    host: &str,
    quic_port: u16,
    wss_port: u16,
    token: &str,
    cert_sha256: Option<&[u8; 32]>,
    width: u32,
    height: u32,
) -> String {
    let mut uri = format!(
        "nve://pair?host={host}&quic={quic_port}&wss={wss_port}&token={token}&w={width}&h={height}"
    );
    if let Some(hash) = cert_sha256 {
        uri.push_str("&cert_hash=");
        uri.push_str(&format_cert_hash_hex(hash));
    }
    uri
}

/// Parses `nve://pair?...`, `quic://host:port`, `https://host:port/path`, `wss://...`, `ws://...`,
/// or bare `host:port` endpoints.
pub fn parse_wire_uri(raw: &str) -> Result<NveWireUri, NveWireError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(NveWireError::InvalidPayload("empty wire URI"));
    }

    if let Some(rest) = trimmed.strip_prefix("nve://") {
        return parse_nve_pair_scheme(rest);
    }

    let (transport, remainder) = if let Some(r) = trimmed.strip_prefix("quic://") {
        (NveWireTransportKind::Quic, r)
    } else if let Some(r) = trimmed.strip_prefix("wt://") {
        (NveWireTransportKind::WebTransport, r)
    } else if let Some(r) = trimmed.strip_prefix("https://") {
        (NveWireTransportKind::WebTransport, r)
    } else if let Some(r) = trimmed.strip_prefix("wss://") {
        (NveWireTransportKind::Wss, r)
    } else if let Some(r) = trimmed.strip_prefix("ws://") {
        (NveWireTransportKind::Ws, r)
    } else {
        (NveWireTransportKind::Auto, trimmed)
    };

    let (authority_and_path, query) = match remainder.split_once('?') {
        Some((a, q)) => (a, Some(q)),
        None => (remainder, None),
    };
    let (authority, path) = match authority_and_path.split_once('/') {
        Some((a, p)) => (a, format!("/{p}")),
        None => (authority_and_path, DEFAULT_WIRE_WSS_PATH.to_owned()),
    };

    let (host, port) = split_host_port(authority)?;
    let (quic_port, wss_port) = match transport {
        NveWireTransportKind::Quic | NveWireTransportKind::WebTransport => (port, None),
        NveWireTransportKind::Wss | NveWireTransportKind::Ws => (None, port),
        NveWireTransportKind::Auto => (port, port),
    };

    let mut uri = NveWireUri {
        host,
        quic_port,
        wss_port,
        path,
        transport,
        token: String::new(),
        cert_sha256: None,
        width: None,
        height: None,
    };
    if let Some(q) = query {
        apply_query_params(&mut uri, q)?;
    }
    Ok(uri)
}

fn parse_nve_pair_scheme(rest: &str) -> Result<NveWireUri, NveWireError> {
    let query = rest
        .split_once('?')
        .map(|(_, q)| q)
        .ok_or(NveWireError::InvalidPayload("nve://pair URI missing query parameters"))?;
    let mut uri = NveWireUri {
        host: String::new(),
        quic_port: None,
        wss_port: None,
        path: DEFAULT_WIRE_WSS_PATH.to_owned(),
        transport: NveWireTransportKind::Auto,
        token: String::new(),
        cert_sha256: None,
        width: None,
        height: None,
    };
    apply_query_params(&mut uri, query)?;
    if uri.host.is_empty() {
        return Err(NveWireError::InvalidPayload("nve://pair URI missing host"));
    }
    if uri.quic_port.is_none() && uri.wss_port.is_none() {
        return Err(NveWireError::InvalidPayload("nve://pair URI missing quic or wss port"));
    }
    Ok(uri)
}

fn apply_query_params(uri: &mut NveWireUri, query: &str) -> Result<(), NveWireError> {
    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let Some((k, v)) = pair.split_once('=') else {
            continue;
        };
        match k {
            "host" => uri.host = v.to_owned(),
            "quic" | "udp" => {
                uri.quic_port = Some(
                    v.parse::<u16>()
                        .map_err(|_| NveWireError::InvalidPayload("invalid quic port"))?,
                );
            }
            "wss" | "ws" | "tcp" => {
                uri.wss_port = Some(
                    v.parse::<u16>()
                        .map_err(|_| NveWireError::InvalidPayload("invalid wss port"))?,
                );
            }
            "path" => uri.path = v.to_owned(),
            "token" => uri.token = v.to_owned(),
            "cert_hash" | "cert" => uri.cert_sha256 = Some(parse_cert_hash_hex(v)?),
            "transport" => uri.transport = NveWireTransportKind::parse(v)?,
            "w" => uri.width = v.parse::<u32>().ok(),
            "h" => uri.height = v.parse::<u32>().ok(),
            _ => {}
        }
    }
    Ok(())
}

fn split_host_port(authority: &str) -> Result<(String, Option<u16>), NveWireError> {
    if authority.is_empty() {
        return Err(NveWireError::InvalidPayload("empty host"));
    }
    if let Some(bracketed) = authority.strip_prefix('[') {
        let (host, rest) = bracketed
            .split_once(']')
            .ok_or(NveWireError::InvalidPayload("unclosed IPv6 host bracket"))?;
        let port = if let Some(port_str) = rest.strip_prefix(':') {
            Some(
                port_str
                    .parse::<u16>()
                    .map_err(|_| NveWireError::InvalidPayload("invalid port"))?,
            )
        } else if rest.is_empty() {
            None
        } else {
            return Err(NveWireError::InvalidPayload("unexpected suffix after IPv6 host"));
        };
        return Ok((host.to_owned(), port));
    }
    if let Some((host, port_str)) = authority.rsplit_once(':')
        && let Ok(port) = port_str.parse::<u16>()
    {
        return Ok((host.to_owned(), Some(port)));
    }
    Ok((authority.to_owned(), None))
}
