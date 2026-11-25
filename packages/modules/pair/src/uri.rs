use std::fmt;

use nve_interop::{DEFAULT_SCHEMA_VERSION, PairingConfig};

/// Transport scheme parsed from a pairing URI.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairTransportKind {
    Ws = 0,
    Wss = 1,
    Tcp = 2,
    Udp = 3,
}

/// Structured pairing endpoint parsed from `nve://pair?...`, `ws://`, `wss://`, `tcp://`,
/// `udp://`, or bare `host:port`.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedPairUri {
    pub transport: PairTransportKind,
    pub host: String,
    pub port: u16,
    pub path: String,
    pub token: String,
    pub schema_version: u16,
    pub udp_port: Option<u16>,
    pub width: Option<f32>,
    pub height: Option<f32>,
}

impl ParsedPairUri {
    pub fn to_pairing_config(&self) -> PairingConfig {
        PairingConfig::new(self.token.clone()).with_schema_version(self.schema_version)
    }

    pub fn endpoint_uri(&self) -> String {
        let scheme = match self.transport {
            PairTransportKind::Ws => "ws",
            PairTransportKind::Wss => "wss",
            PairTransportKind::Tcp => "tcp",
            PairTransportKind::Udp => "udp",
        };
        let host = if self.host.contains(':') && !self.host.starts_with('[') {
            format!("[{}]", self.host)
        } else {
            self.host.clone()
        };
        if self.path.is_empty() {
            format!("{scheme}://{host}:{}", self.port)
        } else {
            format!("{scheme}://{host}:{}{}", self.port, self.path)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairUriError {
    pub message: String,
}

impl PairUriError {
    fn new(msg: impl Into<String>) -> Self {
        Self {
            message: msg.into(),
        }
    }
}

impl fmt::Display for PairUriError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for PairUriError {}

/// Parses any NVE pairing URI (`nve://pair?...`, `ws://`, `wss://`, `tcp://`, `udp://`,
/// or `host:port`) with an optional fallback token.
pub fn parse_pair_uri(
    raw: &str,
    fallback_token: Option<&str>,
) -> Result<ParsedPairUri, PairUriError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(PairUriError::new("pairing URI cannot be empty"));
    }
    let fallback = fallback_token.map(str::trim).filter(|s| !s.is_empty());

    if let Some(rest) = strip_prefix_ascii_ci(trimmed, "nve://") {
        return parse_nve_scheme(rest, fallback);
    }

    let (transport, rest, default_port, default_path) =
        if let Some(r) = strip_prefix_ascii_ci(trimmed, "wss://") {
            (PairTransportKind::Wss, r, Some(443u16), "/ws/nvd")
        } else if let Some(r) = strip_prefix_ascii_ci(trimmed, "ws://") {
            (PairTransportKind::Ws, r, Some(80u16), "/ws/nvd")
        } else if let Some(r) = strip_prefix_ascii_ci(trimmed, "tcp://") {
            (PairTransportKind::Tcp, r, None, "")
        } else if let Some(r) = strip_prefix_ascii_ci(trimmed, "udp://") {
            (PairTransportKind::Udp, r, None, "")
        } else {
            (PairTransportKind::Ws, trimmed, None, "/ws/nvd")
        };

    let (authority_and_path, query) = match rest.split_once('?') {
        Some((a, q)) => (a, Some(q)),
        None => (rest, None),
    };
    let (authority, raw_path) = match authority_and_path.find('/') {
        Some(idx) => (&authority_and_path[..idx], &authority_and_path[idx..]),
        None => (authority_and_path, ""),
    };

    let (host, port) = split_host_port(authority, default_port)?;
    let mut token = fallback.map(str::to_owned);
    let mut schema_version = DEFAULT_SCHEMA_VERSION;
    let mut udp_port = None;
    let mut width = None;
    let mut height = None;

    if let Some(q) = query {
        for (k, v) in parse_query_pairs(q) {
            match k.as_str() {
                "token" if token.is_none() && !v.is_empty() => token = Some(v),
                "schema" if !v.is_empty() => schema_version = parse_positive_u16(&v, "schema")?,
                "udp" if !v.is_empty() => udp_port = Some(parse_positive_u16(&v, "udp")?),
                "w" if !v.is_empty() => width = parse_positive_f32(&v, "w")?,
                "h" if !v.is_empty() => height = parse_positive_f32(&v, "h")?,
                _ => {}
            }
        }
    }

    let token = token.ok_or_else(|| PairUriError::new("missing pairing token"))?;
    let path = if raw_path.is_empty() {
        default_path.to_owned()
    } else {
        raw_path.to_owned()
    };

    Ok(ParsedPairUri {
        transport,
        host,
        port,
        path,
        token,
        schema_version,
        udp_port,
        width,
        height,
    })
}

fn parse_nve_scheme(rest: &str, fallback: Option<&str>) -> Result<ParsedPairUri, PairUriError> {
    let (head, query) = match rest.split_once('?') {
        Some((h, q)) => (h, q),
        None => return Err(PairUriError::new("nve://pair URI requires query parameters")),
    };
    let head_clean = head.trim_end_matches('/');
    if !head_clean.eq_ignore_ascii_case("pair") {
        return Err(PairUriError::new("expected nve://pair URI"));
    }

    let mut ws_param = String::new();
    let mut host_param = String::new();
    let mut token_param = String::new();
    let mut schema_version = DEFAULT_SCHEMA_VERSION;
    let mut udp_port = None;
    let mut width = None;
    let mut height = None;

    for (k, v) in parse_query_pairs(query) {
        match k.as_str() {
            "ws" | "endpoint" | "connect" if ws_param.is_empty() => ws_param = v,
            "host" => host_param = v,
            "token" => token_param = v,
            "schema" if !v.is_empty() => schema_version = parse_positive_u16(&v, "schema")?,
            "udp" if !v.is_empty() => udp_port = Some(parse_positive_u16(&v, "udp")?),
            "w" if !v.is_empty() => width = parse_positive_f32(&v, "w")?,
            "h" if !v.is_empty() => height = parse_positive_f32(&v, "h")?,
            _ => {}
        }
    }

    if ws_param.is_empty() {
        return Err(PairUriError::new("nve://pair URI missing ws/endpoint parameter"));
    }
    let token = match fallback {
        Some(t) => t.to_owned(),
        None if !token_param.is_empty() => token_param,
        None => return Err(PairUriError::new("nve://pair URI missing token")),
    };

    if strip_prefix_ascii_ci(&ws_param, "ws://").is_some()
        || strip_prefix_ascii_ci(&ws_param, "wss://").is_some()
        || strip_prefix_ascii_ci(&ws_param, "tcp://").is_some()
        || strip_prefix_ascii_ci(&ws_param, "udp://").is_some()
    {
        let mut inner = parse_pair_uri(&ws_param, Some(&token))?;
        inner.schema_version = schema_version;
        inner.udp_port = udp_port.or(inner.udp_port);
        inner.width = width.or(inner.width);
        inner.height = height.or(inner.height);
        return Ok(inner);
    }

    let port = parse_positive_u16(&ws_param, "ws")?;
    let host = if host_param.is_empty() {
        "127.0.0.1".to_owned()
    } else {
        host_param
    };
    Ok(ParsedPairUri {
        transport: PairTransportKind::Ws,
        host,
        port,
        path: "/ws/nvd".to_owned(),
        token,
        schema_version,
        udp_port,
        width,
        height,
    })
}

fn split_host_port(authority: &str, default_port: Option<u16>) -> Result<(String, u16), PairUriError> {
    let authority = authority.trim();
    if authority.is_empty() {
        return Err(PairUriError::new("missing host"));
    }
    if let Some(after_open) = authority.strip_prefix('[') {
        let Some(close_idx) = after_open.find(']') else {
            return Err(PairUriError::new("unclosed IPv6 bracket in host"));
        };
        let host = &after_open[..close_idx];
        if host.is_empty() {
            return Err(PairUriError::new("empty IPv6 host"));
        }
        let rest = &after_open[close_idx + 1..];
        let port = if rest.is_empty() {
            default_port.ok_or_else(|| PairUriError::new("missing port"))?
        } else if let Some(p_str) = rest.strip_prefix(':') {
            parse_positive_u16(p_str, "port")?
        } else {
            return Err(PairUriError::new("malformed host:port"));
        };
        return Ok((host.to_owned(), port));
    }

    match authority.rsplit_once(':') {
        Some((host, port_str)) if !host.is_empty() => {
            Ok((host.to_owned(), parse_positive_u16(port_str, "port")?))
        }
        Some(_) => Err(PairUriError::new("empty host")),
        None => {
            let port = default_port.ok_or_else(|| PairUriError::new("missing port"))?;
            Ok((authority.to_owned(), port))
        }
    }
}

fn parse_positive_u16(raw: &str, field: &str) -> Result<u16, PairUriError> {
    let v = raw
        .trim()
        .parse::<u16>()
        .map_err(|_| PairUriError::new(format!("invalid {field} '{raw}'")))?;
    if v == 0 {
        return Err(PairUriError::new(format!("{field} must be >= 1")));
    }
    Ok(v)
}

fn parse_positive_f32(raw: &str, field: &str) -> Result<Option<f32>, PairUriError> {
    let v = raw
        .trim()
        .parse::<f32>()
        .map_err(|_| PairUriError::new(format!("invalid {field} '{raw}'")))?;
    if !v.is_finite() || v <= 0.0 {
        return Err(PairUriError::new(format!("{field} must be finite and > 0")));
    }
    Ok(Some(v))
}

fn strip_prefix_ascii_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    let head = s.get(..prefix.len())?;
    if head.eq_ignore_ascii_case(prefix) {
        Some(&s[prefix.len()..])
    } else {
        None
    }
}

fn parse_query_pairs(query: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (k, v) = match pair.split_once('=') {
            Some((k, v)) => (k, v),
            None => (pair, ""),
        };
        let key = percent_decode(k).trim().to_ascii_lowercase();
        let val = percent_decode(v).trim().to_owned();
        out.push((key, val));
    }
    out
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(hi), Some(lo)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2]))
        {
            out.push((hi << 4) | lo);
            i += 3;
            continue;
        }
        out.push(if bytes[i] == b'+' { b' ' } else { bytes[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}
