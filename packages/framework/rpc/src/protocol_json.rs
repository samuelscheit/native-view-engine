//! Connect-compatible RFC 8259 JSON protocol error encoder and single-pass scanner without
//! `serde_json`.

use crate::status::RpcStatus;

const MAX_JSON_SKIP_DEPTH: usize = 32;

/// Encodes a Connect-compatible JSON protocol error (`{"code":"...","message":"..."}`) with
/// zero `serde_json` dependency and proper RFC 8259 string escaping.
pub fn encode_protocol_error_json(status: RpcStatus, message: &str) -> Vec<u8> {
    let code = status.code_str();
    let mut out = String::with_capacity(code.len() + message.len() + 32);
    out.push_str("{\"code\":\"");
    out.push_str(code);
    out.push_str("\",\"message\":\"");
    push_json_escaped(message, &mut out);
    out.push_str("\"}");
    out.into_bytes()
}

/// Parses a Connect-compatible JSON protocol error body (`{"code":"...","message":"..."}`),
/// preserving specific HTTP statuses (such as `415` vs `400` and `429` vs `413`) when their
/// Connect code matches, and falling back to `http_status` on empty or non-JSON bodies.
pub fn decode_protocol_error_json(http_status: u16, body: &[u8]) -> (RpcStatus, String) {
    let exact_http = RpcStatus::try_from_http_status(http_status).filter(|&s| s != RpcStatus::Ok);
    let fallback_status = exact_http.unwrap_or_else(|| {
        let mapped = RpcStatus::from_http_status(http_status);
        if mapped == RpcStatus::Ok {
            RpcStatus::Internal
        } else {
            mapped
        }
    });
    let Ok(text) = core::str::from_utf8(body) else {
        return (fallback_status, fallback_status.code_str().to_owned());
    };
    let (code_field, message_field) = parse_top_level_error_fields(text);
    let status = match (exact_http, code_field.as_deref()) {
        // Preserve more specific HTTP status (e.g. 429 vs 413, 415 vs 400) when Connect code agrees.
        (Some(http_st), Some(code)) if http_st.code_str() == code => http_st,
        (_, Some(code)) => RpcStatus::from_code_str(code).unwrap_or(fallback_status),
        _ => fallback_status,
    };
    let message = message_field.unwrap_or_else(|| status.code_str().to_owned());
    (status, message)
}

fn push_json_escaped(input: &str, out: &mut String) {
    use core::fmt::Write as _;
    for ch in input.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
}

/// Single-pass top-level JSON object scanner extracting `"code"` and `"message"` string fields
/// without substring-match hazards, handling nested structures and UTF-16 surrogate pairs.
fn parse_top_level_error_fields(json: &str) -> (Option<String>, Option<String>) {
    let mut s = JsonScanner::new(json);
    s.skip_ws();
    if !s.eat_byte(b'{') {
        return (None, None);
    }

    let mut code = None;
    let mut message = None;

    loop {
        s.skip_ws();
        if s.eat_byte(b'}') {
            break;
        }
        let Some(key) = s.parse_string() else {
            break;
        };
        s.skip_ws();
        if !s.eat_byte(b':') {
            break;
        }
        s.skip_ws();

        match key.as_str() {
            "code" if code.is_none() && s.peek() == Some(b'"') => {
                let Some(val) = s.parse_string() else {
                    break;
                };
                code = Some(val);
            }
            "message" if message.is_none() && s.peek() == Some(b'"') => {
                let Some(val) = s.parse_string() else {
                    break;
                };
                message = Some(val);
            }
            _ => {
                if !s.skip_value() {
                    break;
                }
            }
        }

        s.skip_ws();
        if s.eat_byte(b',') {
            continue;
        }
        break;
    }

    (code, message)
}

struct JsonScanner<'a> {
    bytes: &'a [u8],
    src: &'a str,
    pos: usize,
}

impl<'a> JsonScanner<'a> {
    #[inline]
    fn new(src: &'a str) -> Self {
        Self {
            bytes: src.as_bytes(),
            src,
            pos: 0,
        }
    }

    #[inline]
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    #[inline]
    fn skip_ws(&mut self) {
        while let Some(b) = self.peek() {
            if matches!(b, b' ' | b'\t' | b'\r' | b'\n') {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    #[inline]
    fn eat_byte(&mut self, expected: u8) -> bool {
        if self.peek() == Some(expected) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn parse_string(&mut self) -> Option<String> {
        if !self.eat_byte(b'"') {
            return None;
        }
        let mut out = String::new();
        while self.pos < self.bytes.len() {
            let b = self.bytes[self.pos];
            if b == b'"' {
                self.pos += 1;
                return Some(out);
            }
            if b == b'\\' {
                self.pos += 1;
                let esc = *self.bytes.get(self.pos)?;
                self.pos += 1;
                match esc {
                    b'"' => out.push('"'),
                    b'\\' => out.push('\\'),
                    b'/' => out.push('/'),
                    b'b' => out.push('\u{0008}'),
                    b'f' => out.push('\u{000C}'),
                    b'n' => out.push('\n'),
                    b'r' => out.push('\r'),
                    b't' => out.push('\t'),
                    b'u' => {
                        let unit = self.read_hex4()?;
                        if (0xD800..=0xDBFF).contains(&unit) {
                            let saved = self.pos;
                            if self.bytes.get(self.pos..self.pos + 2) == Some(b"\\u") {
                                self.pos += 2;
                                match self.read_hex4() {
                                    Some(low) if (0xDC00..=0xDFFF).contains(&low) => {
                                        let cp = 0x10000
                                            + (((unit as u32 - 0xD800) << 10)
                                                | (low as u32 - 0xDC00));
                                        if let Some(ch) = char::from_u32(cp) {
                                            out.push(ch);
                                        }
                                    }
                                    Some(_) => {
                                        self.pos = saved;
                                        out.push(char::REPLACEMENT_CHARACTER);
                                    }
                                    None => return None,
                                }
                            } else {
                                out.push(char::REPLACEMENT_CHARACTER);
                            }
                        } else if (0xDC00..=0xDFFF).contains(&unit) {
                            out.push(char::REPLACEMENT_CHARACTER);
                        } else if let Some(ch) = char::from_u32(unit as u32) {
                            out.push(ch);
                        }
                    }
                    _ => return None,
                }
            } else {
                let rest = &self.src[self.pos..];
                let ch = rest.chars().next()?;
                out.push(ch);
                self.pos += ch.len_utf8();
            }
        }
        None
    }

    fn read_hex4(&mut self) -> Option<u16> {
        let slice = self.bytes.get(self.pos..self.pos + 4)?;
        let mut val = 0u16;
        for &b in slice {
            let d = match b {
                b'0'..=b'9' => (b - b'0') as u16,
                b'a'..=b'f' => (b - b'a' + 10) as u16,
                b'A'..=b'F' => (b - b'A' + 10) as u16,
                _ => return None,
            };
            val = (val << 4) | d;
        }
        self.pos += 4;
        Some(val)
    }

    fn skip_string(&mut self) -> bool {
        if !self.eat_byte(b'"') {
            return false;
        }
        while self.pos < self.bytes.len() {
            match self.bytes[self.pos] {
                b'"' => {
                    self.pos += 1;
                    return true;
                }
                b'\\' => {
                    if self.pos + 1 >= self.bytes.len() {
                        return false;
                    }
                    self.pos += 2;
                }
                _ => self.pos += 1,
            }
        }
        false
    }

    fn skip_value(&mut self) -> bool {
        let mut depth = 0usize;
        let Some(first) = self.peek() else {
            return false;
        };
        if first == b'"' {
            return self.skip_string();
        }
        if first != b'{' && first != b'[' {
            while let Some(b) = self.peek() {
                if matches!(b, b',' | b'}' | b']' | b' ' | b'\t' | b'\r' | b'\n') {
                    return true;
                }
                self.pos += 1;
            }
            return true;
        }

        while let Some(b) = self.peek() {
            match b {
                b'{' | b'[' => {
                    depth += 1;
                    if depth > MAX_JSON_SKIP_DEPTH {
                        return false;
                    }
                    self.pos += 1;
                }
                b'}' | b']' => {
                    self.pos += 1;
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        return true;
                    }
                }
                b'"' => {
                    if !self.skip_string() {
                        return false;
                    }
                }
                _ => self.pos += 1,
            }
        }
        false
    }
}
