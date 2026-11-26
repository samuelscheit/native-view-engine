//! Zero-dependency Rust tokenizer, string literal unescaper, and interpolation detector.

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum TokenKind<'a> {
    Ident(&'a str),
    Punct(char),
    StringLit { raw: &'a str, is_raw: bool },
    OtherLit(&'a str),
}

#[derive(Debug, Clone)]
pub(super) struct LexResult<'a> {
    pub(super) tokens: Vec<TokenKind<'a>>,
    /// For every `Punct('{')`, `Punct('(')`, or `Punct('[')` at index `i`, stores the index of its
    /// matching closing delimiter (`0` for non-opening tokens).
    pub(super) matching_close: Vec<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum LexError {
    UnterminatedString,
    UnterminatedBlockComment,
    UnterminatedChar,
    UnbalancedDelimiter,
}

impl LexError {
    pub(super) fn as_str(&self) -> &'static str {
        match self {
            Self::UnterminatedString => "unterminated string literal",
            Self::UnterminatedBlockComment => "unterminated block comment",
            Self::UnterminatedChar => "unterminated char literal",
            Self::UnbalancedDelimiter => "unbalanced delimiter",
        }
    }
}

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_' || !b.is_ascii()
}

fn is_ident_continue(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || !b.is_ascii()
}

/// Zero-dependency Rust tokenizer with nested block comment skipping, raw/escaped string support,
/// lifetime vs. char disambiguation, and full delimiter balance verification.
pub(super) fn tokenize(src: &str) -> Result<LexResult<'_>, LexError> {
    let bytes = src.as_bytes();
    let len = bytes.len();
    let mut pos = 0usize;
    let mut tokens = Vec::with_capacity((len / 6).max(16));
    let mut matching_close = Vec::with_capacity((len / 6).max(16));
    let mut delim_stack: Vec<(char, usize)> = Vec::with_capacity(32);

    while pos < len {
        let b = bytes[pos];

        // 1. Skip ASCII & UTF-8 whitespace.
        if b.is_ascii_whitespace() {
            pos += 1;
            continue;
        }
        if !b.is_ascii() {
            let Some(ch) = src[pos..].chars().next() else {
                break;
            };
            if ch.is_whitespace() {
                pos += ch.len_utf8();
                continue;
            }
        }

        // 2. Skip line comments (`// ...`).
        if b == b'/' && pos + 1 < len && bytes[pos + 1] == b'/' {
            pos += 2;
            while pos < len && bytes[pos] != b'\n' {
                pos += 1;
            }
            continue;
        }

        // 3. Skip nested block comments (`/* ... /* ... */ ... */`).
        if b == b'/' && pos + 1 < len && bytes[pos + 1] == b'*' {
            pos += 2;
            let mut depth = 1usize;
            while pos < len && depth > 0 {
                if bytes[pos] == b'/' && pos + 1 < len && bytes[pos + 1] == b'*' {
                    depth += 1;
                    pos += 2;
                } else if bytes[pos] == b'*' && pos + 1 < len && bytes[pos + 1] == b'/' {
                    depth -= 1;
                    pos += 2;
                } else {
                    pos += 1;
                }
            }
            if depth != 0 {
                return Err(LexError::UnterminatedBlockComment);
            }
            continue;
        }

        // 4. Normal string literal (`"..."`).
        if b == b'"' {
            let start = pos;
            pos = scan_quoted_string(bytes, pos)?;
            tokens.push(TokenKind::StringLit {
                raw: &src[start..pos],
                is_raw: false,
            });
            matching_close.push(0);
            continue;
        }

        // 5. Raw string (`r"..."`, `r#"..."#`) or raw identifier (`r#ident`).
        if b == b'r' {
            if let Some(end) = try_scan_raw_string(bytes, pos)? {
                tokens.push(TokenKind::StringLit {
                    raw: &src[pos..end],
                    is_raw: true,
                });
                matching_close.push(0);
                pos = end;
                continue;
            }
            if pos + 2 < len && bytes[pos + 1] == b'#' && is_ident_start(bytes[pos + 2]) {
                let start = pos;
                pos += 2;
                while pos < len && is_ident_continue(bytes[pos]) {
                    pos += 1;
                }
                tokens.push(TokenKind::Ident(&src[start..pos]));
                matching_close.push(0);
                continue;
            }
        }

        // 6. Byte/C string or byte char literals (`b"..."`, `br#"..."#`, `c"..."`, `cr#"..."#`, `b'a'`).
        if b == b'b' || b == b'c' {
            if pos + 1 < len && bytes[pos + 1] == b'"' {
                let start = pos;
                pos = scan_quoted_string(bytes, pos + 1)?;
                tokens.push(TokenKind::OtherLit(&src[start..pos]));
                matching_close.push(0);
                continue;
            }
            if pos + 1 < len
                && bytes[pos + 1] == b'r'
                && let Some(end) = try_scan_raw_string(bytes, pos + 1)?
            {
                tokens.push(TokenKind::OtherLit(&src[pos..end]));
                matching_close.push(0);
                pos = end;
                continue;
            }
            if b == b'b' && pos + 1 < len && bytes[pos + 1] == b'\'' {
                let start = pos;
                pos = scan_char_literal(src, pos + 1)?;
                tokens.push(TokenKind::OtherLit(&src[start..pos]));
                matching_close.push(0);
                continue;
            }
        }

        // 7. Char literal or lifetime (`'a'`, `'\n'`, `'static`).
        if b == b'\'' {
            let start = pos;
            if let Some(end) = try_scan_char_literal(src, pos)? {
                tokens.push(TokenKind::OtherLit(&src[start..end]));
                matching_close.push(0);
                pos = end;
                continue;
            }
            // Lifetime (`'a`, `'static`, `'_`).
            if pos + 1 < len && is_ident_start(bytes[pos + 1]) {
                pos += 2;
                while pos < len && is_ident_continue(bytes[pos]) {
                    pos += 1;
                }
                tokens.push(TokenKind::OtherLit(&src[start..pos]));
                matching_close.push(0);
                continue;
            }
            tokens.push(TokenKind::Punct('\''));
            matching_close.push(0);
            pos += 1;
            continue;
        }

        // 8. Numeric literals (`0`, `42u32`, `3.14f32`, `0xff`, `1e-5`).
        if b.is_ascii_digit() {
            let start = pos;
            pos += 1;
            while pos < len {
                let c = bytes[pos];
                if c.is_ascii_alphanumeric()
                    || c == b'_'
                    || (c == b'.'
                        && pos + 1 < len
                        && bytes[pos + 1] != b'.'
                        && !is_ident_start(bytes[pos + 1]))
                    || ((c == b'+' || c == b'-')
                        && pos > start
                        && (bytes[pos - 1] == b'e' || bytes[pos - 1] == b'E'))
                {
                    pos += 1;
                } else {
                    break;
                }
            }
            tokens.push(TokenKind::OtherLit(&src[start..pos]));
            matching_close.push(0);
            continue;
        }

        // 9. Identifiers / keywords.
        if is_ident_start(b) {
            let start = pos;
            let Some(ch) = src[pos..].chars().next() else {
                break;
            };
            pos += ch.len_utf8();
            while pos < len && is_ident_continue(bytes[pos]) {
                let Some(c) = src[pos..].chars().next() else {
                    break;
                };
                pos += c.len_utf8();
            }
            tokens.push(TokenKind::Ident(&src[start..pos]));
            matching_close.push(0);
            continue;
        }

        // 10. Punctuation + delimiter balancing.
        let Some(ch) = src[pos..].chars().next() else {
            break;
        };
        let tok_idx = tokens.len();
        match ch {
            '{' | '(' | '[' => {
                delim_stack.push((ch, tok_idx));
            }
            '}' | ')' | ']' => {
                let expected = match ch {
                    '}' => '{',
                    ')' => '(',
                    ']' => '[',
                    _ => unreachable!(),
                };
                match delim_stack.pop() {
                    Some((open_ch, open_idx)) if open_ch == expected => {
                        matching_close[open_idx] = tok_idx;
                    }
                    _ => return Err(LexError::UnbalancedDelimiter),
                }
            }
            _ => {}
        }
        tokens.push(TokenKind::Punct(ch));
        matching_close.push(0);
        pos += ch.len_utf8();
    }

    if !delim_stack.is_empty() {
        return Err(LexError::UnbalancedDelimiter);
    }

    Ok(LexResult {
        tokens,
        matching_close,
    })
}

fn scan_quoted_string(bytes: &[u8], open_quote: usize) -> Result<usize, LexError> {
    let mut pos = open_quote + 1;
    let len = bytes.len();
    while pos < len {
        match bytes[pos] {
            b'\\' => {
                if pos + 1 >= len {
                    return Err(LexError::UnterminatedString);
                }
                pos += 2;
            }
            b'"' => return Ok(pos + 1),
            _ => pos += 1,
        }
    }
    Err(LexError::UnterminatedString)
}

/// Scans `r"..."` or `r#"..."#` starting at `r_pos`. Returns `Ok(None)` if not a raw string.
fn try_scan_raw_string(bytes: &[u8], r_pos: usize) -> Result<Option<usize>, LexError> {
    let len = bytes.len();
    let mut pos = r_pos + 1;
    let mut hashes = 0usize;
    while pos < len && bytes[pos] == b'#' {
        hashes += 1;
        pos += 1;
    }
    if pos >= len || bytes[pos] != b'"' {
        return Ok(None);
    }
    pos += 1; // consume opening `"`
    while pos < len {
        if bytes[pos] == b'"' {
            let mut matched = 0usize;
            while matched < hashes && pos + 1 + matched < len && bytes[pos + 1 + matched] == b'#' {
                matched += 1;
            }
            if matched == hashes {
                return Ok(Some(pos + 1 + hashes));
            }
        }
        pos += 1;
    }
    Err(LexError::UnterminatedString)
}

fn scan_char_literal(src: &str, quote_pos: usize) -> Result<usize, LexError> {
    try_scan_char_literal(src, quote_pos)?.ok_or(LexError::UnterminatedChar)
}

fn try_scan_char_literal(src: &str, quote_pos: usize) -> Result<Option<usize>, LexError> {
    let bytes = src.as_bytes();
    let len = bytes.len();
    let mut pos = quote_pos + 1;
    if pos >= len {
        return Err(LexError::UnterminatedChar);
    }
    if bytes[pos] == b'\\' {
        // Escaped char literal (`'\n'`, `'\''`, `'\x7f'`, `'\u{1F600}'`).
        pos += 2;
        while pos < len && bytes[pos] != b'\'' && bytes[pos] != b'\n' {
            pos += 1;
        }
        if pos < len && bytes[pos] == b'\'' {
            return Ok(Some(pos + 1));
        }
        return Err(LexError::UnterminatedChar);
    }
    let Some(ch) = src[pos..].chars().next() else {
        return Err(LexError::UnterminatedChar);
    };
    let after_ch = pos + ch.len_utf8();
    if after_ch < len && bytes[after_ch] == b'\'' {
        return Ok(Some(after_ch + 1));
    }
    Ok(None)
}

/// Unescapes a Rust normal (`"..."`) or raw (`r#"..."#`) string literal into its string value.
pub(super) fn unescape_string_literal(raw: &str, is_raw: bool) -> Result<String, LexError> {
    if is_raw {
        let open = raw.find('"').ok_or(LexError::UnterminatedString)?;
        let close = raw.rfind('"').ok_or(LexError::UnterminatedString)?;
        if close <= open {
            return Err(LexError::UnterminatedString);
        }
        return Ok(raw[open + 1..close].to_owned());
    }

    if raw.len() < 2 || !raw.starts_with('"') || !raw.ends_with('"') {
        return Err(LexError::UnterminatedString);
    }
    let inner = &raw[1..raw.len() - 1];
    if !inner.contains('\\') {
        return Ok(inner.to_owned());
    }

    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        let Some(esc) = chars.next() else {
            return Err(LexError::UnterminatedString);
        };
        match esc {
            'n' => out.push('\n'),
            'r' => out.push('\r'),
            't' => out.push('\t'),
            '0' => out.push('\0'),
            '\\' => out.push('\\'),
            '"' => out.push('"'),
            '\'' => out.push('\''),
            '\n' | '\r' => {
                // Rust string line continuation: skip leading ASCII whitespace on next line.
                while let Some(&next_ch) = chars.peek() {
                    if next_ch == ' ' || next_ch == '\t' || next_ch == '\n' || next_ch == '\r' {
                        chars.next();
                    } else {
                        break;
                    }
                }
            }
            'x' => {
                let h1 = chars.next().ok_or(LexError::UnterminatedString)?;
                let h2 = chars.next().ok_or(LexError::UnterminatedString)?;
                let mut buf = [0u8; 2];
                let hex = format!("{}{}", h1.encode_utf8(&mut buf[..1]), h2);
                let byte = u8::from_str_radix(&hex, 16).map_err(|_| LexError::UnterminatedString)?;
                out.push(byte as char);
            }
            'u' => {
                if chars.next() != Some('{') {
                    return Err(LexError::UnterminatedString);
                }
                let mut hex = String::with_capacity(8);
                let mut closed = false;
                for ch in chars.by_ref() {
                    if ch == '}' {
                        closed = true;
                        break;
                    }
                    if ch != '_' {
                        hex.push(ch);
                    }
                }
                if !closed || hex.is_empty() {
                    return Err(LexError::UnterminatedString);
                }
                let code =
                    u32::from_str_radix(&hex, 16).map_err(|_| LexError::UnterminatedString)?;
                let decoded = char::from_u32(code).ok_or(LexError::UnterminatedString)?;
                out.push(decoded);
            }
            other => {
                out.push('\\');
                out.push(other);
            }
        }
    }
    Ok(out)
}

/// Returns `true` if `s` contains any unescaped `{` or `}` format interpolation brace.
/// Escaped `{{` and `}}` represent literal braces and do not count as interpolation.
pub(super) fn has_interpolation_braces(s: &str) -> bool {
    let bytes = s.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'{' => {
                if i + 1 < bytes.len() && bytes[i + 1] == b'{' {
                    i += 2;
                } else {
                    return true;
                }
            }
            b'}' => {
                if i + 1 < bytes.len() && bytes[i + 1] == b'}' {
                    i += 2;
                } else {
                    return true;
                }
            }
            _ => i += 1,
        }
    }
    false
}
