//! Zero-dependency Rust tokenizer producing compact struct-of-arrays tokens (13 B/token: a kind
//! byte plus `u32` start/end offsets and matching-delimiter index), with nested block comments,
//! raw/escaped strings, lifetime vs. char disambiguation and delimiter balance verification.

/// Token class; the token text is `src[start..end]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub(super) enum Tok {
    Ident,
    Punct,
    /// `"..."` string literal.
    Str,
    /// `r"..."` / `r#"..."#` raw string literal.
    RawStr,
    /// Integer or float literal (any radix, `_` separators, suffix).
    Number,
    /// Char, byte, byte-string and C-string literals, lifetimes.
    OtherLit,
}

/// Owned token table of one source file (no borrow of the source, so it can be cached).
#[derive(Debug, Clone, Default)]
pub struct Tokens {
    kinds: Vec<Tok>,
    starts: Vec<u32>,
    ends: Vec<u32>,
    /// Matching closing-delimiter index for `{`, `(`, `[` (`0` for every other token).
    close: Vec<u32>,
}

impl Tokens {
    fn with_capacity(cap: usize) -> Self {
        Self {
            kinds: Vec::with_capacity(cap),
            starts: Vec::with_capacity(cap),
            ends: Vec::with_capacity(cap),
            close: Vec::with_capacity(cap),
        }
    }

    #[inline]
    fn push(&mut self, kind: Tok, start: usize, end: usize) {
        self.kinds.push(kind);
        self.starts.push(start as u32);
        self.ends.push(end as u32);
        self.close.push(0);
    }
}

/// A token table paired with the source it was lexed from.
#[derive(Debug, Clone, Copy)]
pub(super) struct Lex<'a> {
    pub(super) src: &'a str,
    tokens: &'a Tokens,
}

impl<'a> Lex<'a> {
    pub(super) fn new(src: &'a str, tokens: &'a Tokens) -> Self {
        Self { src, tokens }
    }

    #[inline]
    pub(super) fn len(self) -> usize {
        self.tokens.kinds.len()
    }

    #[inline]
    pub(super) fn kind(self, i: usize) -> Tok {
        self.tokens.kinds[i]
    }

    #[inline]
    pub(super) fn start(self, i: usize) -> usize {
        self.tokens.starts[i] as usize
    }

    #[inline]
    pub(super) fn text(self, i: usize) -> &'a str {
        let (s, e) = (self.tokens.starts[i] as usize, self.tokens.ends[i] as usize);
        &self.src[s..e]
    }

    /// Index of the closing delimiter matching the opener at `i`.
    #[inline]
    pub(super) fn close(self, i: usize) -> usize {
        self.tokens.close[i] as usize
    }

    /// Whether token `i` exists and is the ASCII punctuation `c`.
    #[inline]
    pub(super) fn is_punct(self, i: usize, c: u8) -> bool {
        i < self.len() && self.kind(i) == Tok::Punct && self.src.as_bytes()[self.start(i)] == c
    }

    /// Whether token `i` exists and is the identifier `name`.
    #[inline]
    pub(super) fn is_ident(self, i: usize, name: &str) -> bool {
        i < self.len() && self.kind(i) == Tok::Ident && self.text(i) == name
    }

    #[inline]
    pub(super) fn ident(self, i: usize) -> Option<&'a str> {
        (i < self.len() && self.kind(i) == Tok::Ident).then(|| self.text(i))
    }

    /// `(raw source text, is_raw)` of a string literal token.
    #[inline]
    pub(super) fn string_lit(self, i: usize) -> Option<(&'a str, bool)> {
        match self.kind(i) {
            Tok::Str => Some((self.text(i), false)),
            Tok::RawStr => Some((self.text(i), true)),
            _ => None,
        }
    }

    /// Token `i` of `self` equals token `j` of `other` (kind and text).
    #[inline]
    pub(super) fn same(self, i: usize, other: Lex<'_>, j: usize) -> bool {
        self.kind(i) == other.kind(j) && self.text(i) == other.text(j)
    }

    /// `self[i..i_end]` equals `other[j..j + (i_end - i)]` (false when `other` is too short).
    pub(super) fn same_range(self, i: usize, i_end: usize, other: Lex<'_>, j: usize) -> bool {
        let n = i_end - i;
        j + n <= other.len()
            && self.tokens.kinds[i..i_end] == other.tokens.kinds[j..j + n]
            && (0..n).all(|k| self.text(i + k) == other.text(j + k))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LexError {
    UnterminatedString,
    UnterminatedBlockComment,
    UnterminatedChar,
    UnbalancedDelimiter,
    InvalidEscape,
    SourceTooLarge,
}

impl LexError {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::UnterminatedString => "unterminated string literal",
            Self::UnterminatedBlockComment => "unterminated block comment",
            Self::UnterminatedChar => "unterminated char literal",
            Self::UnbalancedDelimiter => "unbalanced delimiter",
            Self::InvalidEscape => "invalid string escape",
            Self::SourceTooLarge => "source file exceeds 4 GiB",
        }
    }
}

/// ASCII identifier start, or any non-ASCII lead byte (refined per char by [`ident_char`]).
fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_' || !b.is_ascii()
}

fn is_ident_continue(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Width of the identifier char at `pos` (`XID`-approximated for non-ASCII), or `None`.
fn ident_char(src: &str, pos: usize, first: bool) -> Option<usize> {
    let b = *src.as_bytes().get(pos)?;
    if b.is_ascii() {
        return (if first {
            is_ident_start(b)
        } else {
            is_ident_continue(b)
        })
        .then_some(1);
    }
    let c = src[pos..].chars().next()?;
    (c.is_alphanumeric() && !(first && c.is_numeric())).then(|| c.len_utf8())
}

/// Tokenizes `src`. Initial capacity `len / 4` covers dense Rust (one token per ~4-5 bytes) so
/// the four columns rarely reallocate.
pub(super) fn tokenize(src: &str) -> Result<Tokens, LexError> {
    let bytes = src.as_bytes();
    let len = bytes.len();
    if len > u32::MAX as usize {
        return Err(LexError::SourceTooLarge);
    }
    let mut pos = 0usize;
    let mut out = Tokens::with_capacity((len / 4).max(16));
    let mut delim_stack: Vec<(u8, u32)> = Vec::with_capacity(64);

    while pos < len {
        let b = bytes[pos];
        let start = pos;

        // Whitespace (ASCII and Unicode).
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

        // Line comment.
        if b == b'/' && bytes.get(pos + 1) == Some(&b'/') {
            while pos < len && bytes[pos] != b'\n' {
                pos += 1;
            }
            continue;
        }

        // Nested block comment.
        if b == b'/' && bytes.get(pos + 1) == Some(&b'*') {
            pos += 2;
            let mut depth = 1usize;
            while pos < len && depth > 0 {
                match (bytes[pos], bytes.get(pos + 1)) {
                    (b'/', Some(b'*')) => (depth, pos) = (depth + 1, pos + 2),
                    (b'*', Some(b'/')) => (depth, pos) = (depth - 1, pos + 2),
                    _ => pos += 1,
                }
            }
            if depth != 0 {
                return Err(LexError::UnterminatedBlockComment);
            }
            continue;
        }

        if b == b'"' {
            pos = scan_quoted_string(bytes, pos)?;
            out.push(Tok::Str, start, pos);
            continue;
        }

        // Raw string (`r"..."`, `r#"..."#`) or raw identifier (`r#ident`).
        if b == b'r' {
            if let Some(end) = try_scan_raw_string(bytes, pos)? {
                out.push(Tok::RawStr, start, end);
                pos = end;
                continue;
            }
            if bytes.get(pos + 1) == Some(&b'#')
                && bytes.get(pos + 2).is_some_and(|&c| is_ident_start(c))
            {
                pos += 2;
                while pos < len && is_ident_continue(bytes[pos]) {
                    pos += 1;
                }
                out.push(Tok::Ident, start, pos);
                continue;
            }
        }

        // Byte / C strings and byte chars (`b"..."`, `br#"..."#`, `c"..."`, `b'a'`).
        if b == b'b' || b == b'c' {
            let end = if bytes.get(pos + 1) == Some(&b'"') {
                Some(scan_quoted_string(bytes, pos + 1)?)
            } else if bytes.get(pos + 1) == Some(&b'r') {
                try_scan_raw_string(bytes, pos + 1)?
            } else if b == b'b' && bytes.get(pos + 1) == Some(&b'\'') {
                Some(try_scan_char_literal(src, pos + 1)?.ok_or(LexError::UnterminatedChar)?)
            } else {
                None
            };
            if let Some(end) = end {
                out.push(Tok::OtherLit, start, end);
                pos = end;
                continue;
            }
        }

        // Char literal or lifetime (`'a'`, `'\n'`, `'static`).
        if b == b'\'' {
            if let Some(end) = try_scan_char_literal(src, pos)? {
                out.push(Tok::OtherLit, start, end);
                pos = end;
                continue;
            }
            if bytes.get(pos + 1).is_some_and(|&c| is_ident_start(c)) {
                pos += 2;
                while pos < len && is_ident_continue(bytes[pos]) {
                    pos += 1;
                }
                out.push(Tok::OtherLit, start, pos);
                continue;
            }
            out.push(Tok::Punct, start, pos + 1);
            pos += 1;
            continue;
        }

        if b.is_ascii_digit() {
            pos = scan_number(bytes, pos);
            out.push(Tok::Number, start, pos);
            continue;
        }

        if let Some(width) = ident_char(src, pos, true) {
            pos += width;
            while let Some(width) = ident_char(src, pos, false) {
                pos += width;
            }
            out.push(Tok::Ident, start, pos);
            continue;
        }

        // Punctuation + delimiter balancing.
        let width = src[pos..].chars().next().map_or(1, char::len_utf8);
        let idx = out.kinds.len() as u32;
        match b {
            b'{' | b'(' | b'[' => delim_stack.push((b, idx)),
            b'}' | b')' | b']' => {
                let expected = match b {
                    b'}' => b'{',
                    b')' => b'(',
                    _ => b'[',
                };
                match delim_stack.pop() {
                    Some((open, open_idx)) if open == expected => {
                        out.close[open_idx as usize] = idx;
                    }
                    _ => return Err(LexError::UnbalancedDelimiter),
                }
            }
            _ => {}
        }
        out.push(Tok::Punct, start, pos + width);
        pos += width;
    }

    if !delim_stack.is_empty() {
        return Err(LexError::UnbalancedDelimiter);
    }
    Ok(out)
}

/// Scans a number literal from its first digit. `.` and signed exponents belong only to decimal
/// literals (`0xfe-1` is `0xfe`, `-`, `1`); a suffix (`u8`, `f32`) ends both.
fn scan_number(bytes: &[u8], mut pos: usize) -> usize {
    let len = bytes.len();
    let radix_prefixed =
        bytes[pos] == b'0' && matches!(bytes.get(pos + 1), Some(b'x' | b'o' | b'b'));
    pos += if radix_prefixed { 2 } else { 1 };
    let decimal = !radix_prefixed;
    let (mut seen_dot, mut seen_exp, mut in_suffix) = (false, false, false);
    while pos < len {
        let c = bytes[pos];
        let next = bytes.get(pos + 1).copied();
        if c.is_ascii_digit() || c == b'_' {
            pos += 1;
        } else if c.is_ascii_alphabetic() {
            let exponent = decimal
                && !in_suffix
                && !seen_exp
                && (c == b'e' || c == b'E')
                && next.is_some_and(|n| n.is_ascii_digit() || n == b'_' || n == b'+' || n == b'-');
            if exponent {
                seen_exp = true;
            } else {
                in_suffix = true;
            }
            pos += 1;
        } else if (c == b'+' || c == b'-')
            && decimal
            && seen_exp
            && !in_suffix
            && matches!(bytes[pos - 1], b'e' | b'E')
            && next.is_some_and(|n| n.is_ascii_digit())
        {
            pos += 1;
        } else if c == b'.'
            && decimal
            && !seen_dot
            && !seen_exp
            && !in_suffix
            && next.is_none_or(|n| n != b'.' && !is_ident_start(n))
        {
            seen_dot = true;
            pos += 1;
        } else {
            break;
        }
    }
    pos
}

fn scan_quoted_string(bytes: &[u8], open_quote: usize) -> Result<usize, LexError> {
    let mut pos = open_quote + 1;
    while pos < bytes.len() {
        match bytes[pos] {
            b'\\' => pos += 2,
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
    pos += 1;
    while pos < len {
        if bytes[pos] == b'"'
            && bytes
                .get(pos + 1..pos + 1 + hashes)
                .is_some_and(|h| h.iter().all(|&c| c == b'#'))
        {
            return Ok(Some(pos + 1 + hashes));
        }
        pos += 1;
    }
    Err(LexError::UnterminatedString)
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
        return if pos < len && bytes[pos] == b'\'' {
            Ok(Some(pos + 1))
        } else {
            Err(LexError::UnterminatedChar)
        };
    }
    let Some(ch) = src[pos..].chars().next() else {
        return Err(LexError::UnterminatedChar);
    };
    let after = pos + ch.len_utf8();
    Ok((bytes.get(after) == Some(&b'\'')).then_some(after + 1))
}
