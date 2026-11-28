//! Minimal zero-dependency Rust lexer matching `nve-cli`'s `rsx_diff/lexer.rs` token stream.

/// 1-based `(line, column)` location matching `proc_macro::Span::{line, column}`.
pub(crate) type LineCol = (u32, u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tok {
    Ident,
    Str,
    RawStr,
    Number,
    OtherLit,
    Punct,
}

pub(crate) struct Token {
    pub(crate) kind: Tok,
    pub(crate) start: usize,
    pub(crate) end: usize,
    /// Closing delimiter token index for `{`, `(`, `[` (0 otherwise).
    pub(crate) close: usize,
}

pub(crate) struct Lex<'a> {
    pub(crate) src: &'a str,
    pub(crate) tokens: Vec<Token>,
    line_starts: Vec<usize>,
}

impl<'a> Lex<'a> {
    pub(crate) fn tokenize(src: &'a str) -> Option<Self> {
        let bytes = src.as_bytes();
        let mut line_starts = vec![0usize];
        for (i, &b) in bytes.iter().enumerate() {
            if b == b'\n' {
                line_starts.push(i + 1);
            }
        }
        let mut tokens = Vec::with_capacity(src.len() / 5);
        let mut stack: Vec<(u8, usize)> = Vec::new();
        let mut i = 0usize;
        let len = bytes.len();

        while i < len {
            let b = bytes[i];
            if b.is_ascii_whitespace() {
                i += 1;
                continue;
            }
            if b == b'/' && i + 1 < len {
                if bytes[i + 1] == b'/' {
                    i += 2;
                    while i < len && bytes[i] != b'\n' {
                        i += 1;
                    }
                    continue;
                }
                if bytes[i + 1] == b'*' {
                    i += 2;
                    let mut depth = 1u32;
                    while i + 1 < len && depth > 0 {
                        if bytes[i] == b'/' && bytes[i + 1] == b'*' {
                            depth += 1;
                            i += 2;
                        } else if bytes[i] == b'*' && bytes[i + 1] == b'/' {
                            depth -= 1;
                            i += 2;
                        } else {
                            i += 1;
                        }
                    }
                    if depth != 0 {
                        return None;
                    }
                    continue;
                }
            }
            // Raw strings: `r#"..."#`, `br#"..."#`, `cr#"..."#`.
            if let Some((end, is_plain_raw)) = scan_raw_string(bytes, i) {
                tokens.push(Token {
                    kind: if is_plain_raw { Tok::RawStr } else { Tok::OtherLit },
                    start: i,
                    end,
                    close: 0,
                });
                i = end;
                continue;
            }
            // Byte/C normal strings: `b"..."`, `c"..."`.
            if (b == b'b' || b == b'c') && i + 1 < len && bytes[i + 1] == b'"' {
                let end = scan_quoted(bytes, i + 1, b'"')?;
                tokens.push(Token {
                    kind: Tok::OtherLit,
                    start: i,
                    end,
                    close: 0,
                });
                i = end;
                continue;
            }
            // Byte char literal `b'x'`.
            if b == b'b' && i + 1 < len && bytes[i + 1] == b'\'' {
                let end = scan_char_lit(bytes, i + 1)?;
                tokens.push(Token {
                    kind: Tok::OtherLit,
                    start: i,
                    end,
                    close: 0,
                });
                i = end;
                continue;
            }
            // Normal string literal.
            if b == b'"' {
                let end = scan_quoted(bytes, i, b'"')?;
                tokens.push(Token {
                    kind: Tok::Str,
                    start: i,
                    end,
                    close: 0,
                });
                i = end;
                continue;
            }
            // Char literal vs lifetime.
            if b == b'\'' {
                if let Some(end) = scan_char_lit(bytes, i) {
                    tokens.push(Token {
                        kind: Tok::OtherLit,
                        start: i,
                        end,
                        close: 0,
                    });
                    i = end;
                } else {
                    let start = i;
                    i += 1;
                    while i < len && is_ident_continue(bytes[i]) {
                        i += 1;
                    }
                    tokens.push(Token {
                        kind: Tok::OtherLit,
                        start,
                        end: i,
                        close: 0,
                    });
                }
                continue;
            }
            // Raw identifier `r#ident`.
            if b == b'r'
                && i + 2 < len
                && bytes[i + 1] == b'#'
                && is_ident_start(bytes[i + 2])
            {
                let start = i;
                i += 2;
                while i < len && is_ident_continue(bytes[i]) {
                    i += 1;
                }
                tokens.push(Token {
                    kind: Tok::Ident,
                    start,
                    end: i,
                    close: 0,
                });
                continue;
            }
            // Identifier (ASCII or UTF-8 non-ASCII).
            if is_ident_start(b) || b >= 0x80 {
                let start = i;
                if b >= 0x80 {
                    let mut chars = src[i..].char_indices();
                    if let Some((_, ch)) = chars.next() {
                        i += ch.len_utf8();
                        for (_, ch) in chars {
                            if ch.is_alphanumeric() || ch == '_' {
                                i += ch.len_utf8();
                            } else {
                                break;
                            }
                        }
                    }
                } else {
                    i += 1;
                    while i < len && is_ident_continue(bytes[i]) {
                        i += 1;
                    }
                }
                tokens.push(Token {
                    kind: Tok::Ident,
                    start,
                    end: i,
                    close: 0,
                });
                continue;
            }
            // Number literal.
            if b.is_ascii_digit() {
                let start = i;
                i += 1;
                while i < len {
                    let c = bytes[i];
                    if c.is_ascii_alphanumeric() || c == b'_' {
                        i += 1;
                    } else if c == b'.'
                        && i + 1 < len
                        && bytes[i + 1].is_ascii_digit()
                    {
                        i += 1;
                    } else if (c == b'+' || c == b'-')
                        && i > start
                        && matches!(bytes[i - 1], b'e' | b'E')
                    {
                        i += 1;
                    } else {
                        break;
                    }
                }
                tokens.push(Token {
                    kind: Tok::Number,
                    start,
                    end: i,
                    close: 0,
                });
                continue;
            }
            // Single-byte punctuation (with delimiter matching).
            let tok_idx = tokens.len();
            match b {
                b'{' | b'(' | b'[' => stack.push((b, tok_idx)),
                b'}' | b')' | b']' => {
                    let expected = match b {
                        b'}' => b'{',
                        b')' => b'(',
                        _ => b'[',
                    };
                    let (open_ch, open_idx) = stack.pop()?;
                    if open_ch != expected {
                        return None;
                    }
                    tokens[open_idx].close = tok_idx;
                }
                _ => {}
            }
            tokens.push(Token {
                kind: Tok::Punct,
                start: i,
                end: i + 1,
                close: 0,
            });
            i += 1;
        }
        if !stack.is_empty() {
            return None;
        }
        Some(Self {
            src,
            tokens,
            line_starts,
        })
    }

    pub(crate) fn len(&self) -> usize {
        self.tokens.len()
    }

    pub(crate) fn kind(&self, idx: usize) -> Tok {
        self.tokens[idx].kind
    }

    pub(crate) fn text(&self, idx: usize) -> &'a str {
        let t = &self.tokens[idx];
        &self.src[t.start..t.end]
    }

    pub(crate) fn ident(&self, idx: usize) -> Option<&'a str> {
        (idx < self.tokens.len() && self.tokens[idx].kind == Tok::Ident)
            .then(|| self.text(idx))
    }

    pub(crate) fn is_ident(&self, idx: usize, s: &str) -> bool {
        self.ident(idx) == Some(s)
    }

    pub(crate) fn is_punct(&self, idx: usize, ch: u8) -> bool {
        idx < self.tokens.len()
            && self.tokens[idx].kind == Tok::Punct
            && self.src.as_bytes()[self.tokens[idx].start] == ch
    }

    pub(crate) fn close(&self, open: usize) -> usize {
        self.tokens[open].close
    }

    fn line_col(&self, byte_offset: usize) -> LineCol {
        let line_idx = self
            .line_starts
            .partition_point(|&s| s <= byte_offset)
            .saturating_sub(1);
        let line_start = self.line_starts[line_idx];
        let col = self.src[line_start..byte_offset].chars().count() + 1;
        ((line_idx + 1) as u32, col as u32)
    }

    pub(crate) fn tok_start_lc(&self, idx: usize) -> LineCol {
        self.line_col(self.tokens[idx].start)
    }

    pub(crate) fn tok_end_lc(&self, idx: usize) -> LineCol {
        self.line_col(self.tokens[idx].end)
    }
}

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

fn is_ident_continue(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn scan_quoted(bytes: &[u8], open: usize, quote: u8) -> Option<usize> {
    let mut i = open + 1;
    let len = bytes.len();
    while i < len {
        let b = bytes[i];
        if b == b'\\' {
            i += 2;
            continue;
        }
        if b == quote {
            return Some(i + 1);
        }
        i += 1;
    }
    None
}

fn scan_char_lit(bytes: &[u8], open: usize) -> Option<usize> {
    let len = bytes.len();
    let i = open + 1;
    if i >= len {
        return None;
    }
    if bytes[i] == b'\\' {
        // Escaped char: `'\''`, `'\n'`, `'\x7f'`, `'\u{1234}'`.
        let mut j = i + 2;
        while j < len && bytes[j] != b'\'' && bytes[j] != b'\n' && j - open <= 14 {
            j += 1;
        }
        return (j < len && bytes[j] == b'\'').then_some(j + 1);
    }
    // Single UTF-8 scalar followed immediately by `'`.
    let tail = std::str::from_utf8(&bytes[i..]).ok()?;
    let ch = tail.chars().next()?;
    let after = i + ch.len_utf8();
    (after < len && bytes[after] == b'\'').then_some(after + 1)
}

fn scan_raw_string(bytes: &[u8], start: usize) -> Option<(usize, bool)> {
    let len = bytes.len();
    let (mut i, is_plain) = if bytes[start] == b'r' {
        (start + 1, true)
    } else if (bytes[start] == b'b' || bytes[start] == b'c')
        && start + 1 < len
        && bytes[start + 1] == b'r'
    {
        (start + 2, false)
    } else {
        return None;
    };
    let mut hashes = 0usize;
    while i < len && bytes[i] == b'#' {
        hashes += 1;
        i += 1;
    }
    if i >= len || bytes[i] != b'"' {
        return None;
    }
    i += 1;
    while i < len {
        if bytes[i] == b'"' {
            let mut h = 0usize;
            while h < hashes && i + 1 + h < len && bytes[i + 1 + h] == b'#' {
                h += 1;
            }
            if h == hashes {
                return Some((i + 1 + hashes, is_plain));
            }
        }
        i += 1;
    }
    None
}
