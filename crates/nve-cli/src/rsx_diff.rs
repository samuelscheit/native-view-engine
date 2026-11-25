//! Zero-dependency Rust lexer and `rsx! { ... }` token differ for Tier-1 (`< 10ms`) hot-patching.
//!
//! Compares the cached baseline `.rs` source against the newly saved source:
//! - Ignores comments and whitespace (comment/whitespace-only edits return [`DiffOutcome::NoOp`]).
//! - Verifies all non-string tokens outside and inside `rsx! { ... }` are 100% identical, and that
//!   delimiter nesting and literals are syntactically valid.
//! - Emits [`DiffOutcome::Tier1Patch`] with targeted `target_test_id`, `node_path`,
//!   `text_replacements`, `text_updates`, and `attr_updates` when ONLY pure static string literals
//!   (no `{expr}` interpolation braces) inside `rsx! { ... }` changed.
//! - Immediately falls back to [`DiffOutcome::RequireTier2Rebuild`] for any Rust logic, control
//!   flow, event handler, interpolated literal, structural/key/`test_id`, or syntax change.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use nve_engine::HotPatchMsg;

/// Classification of a watched `.rs` file save for 2-tier hot reload.
#[derive(Debug, Clone, PartialEq)]
pub enum DiffOutcome {
    /// Comment-only, whitespace-only, or byte-identical save (no patch or rebuild needed).
    NoOp,
    /// Pure static text/style literal edit inside `rsx! { ... }` — broadcast via Tier-1 WebSocket.
    Tier1Patch(HotPatchMsg),
    /// Any logic, structural, interpolation, multi-element attr, or syntax change — run Tier-2 build.
    RequireTier2Rebuild { reason: String },
}

/// In-memory cache of the last-known baseline source text of every watched `.rs` file.
#[derive(Debug, Default)]
pub struct RsxWatchCache {
    files: HashMap<PathBuf, String>,
}

impl RsxWatchCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Recursively loads all `.rs` files under `dir` into the cache as the Tier-1 diff baseline.
    pub fn seed_dir(&mut self, dir: &Path) {
        fn walk(dir: &Path, out: &mut HashMap<PathBuf, String>) {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                        if name == "target" || name.starts_with('.') {
                            continue;
                        }
                    }
                    walk(&path, out);
                } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                    if let Ok(src) = std::fs::read_to_string(&path) {
                        out.insert(normalize_cache_key(&path), src);
                    }
                }
            }
        }
        walk(dir, &mut self.files);
    }

    /// Records or updates the baseline source text for a single file.
    #[cfg(test)]
    pub fn update_file(&mut self, path: impl AsRef<Path>, source: String) {
        self.files
            .insert(normalize_cache_key(path.as_ref()), source);
    }

    /// Returns the cached baseline source for `path`, if present.
    #[cfg(test)]
    pub fn cached_source(&self, path: &Path) -> Option<&str> {
        self.files
            .get(&normalize_cache_key(path))
            .map(String::as_str)
    }

    /// Number of `.rs` files currently cached.
    pub fn len(&self) -> usize {
        self.files.len()
    }

    /// Diffs a saved file against its cached baseline.
    ///
    /// Updates the cache on [`DiffOutcome::NoOp`] and [`DiffOutcome::Tier1Patch`] so consecutive
    /// Tier-1 saves chain cleanly. Leaves the cache unchanged on
    /// [`DiffOutcome::RequireTier2Rebuild`] so callers update it once the Tier-2 build succeeds
    /// (preventing broken intermediate syntax from poisoning the baseline).
    pub fn diff_file(&mut self, path: &Path, new_source: &str, generation: u64) -> DiffOutcome {
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            return DiffOutcome::RequireTier2Rebuild {
                reason: format!("non-Rust file changed: {}", path.display()),
            };
        }

        let key = normalize_cache_key(path);
        let Some(old_source) = self.files.get(&key) else {
            return DiffOutcome::RequireTier2Rebuild {
                reason: format!("new uncached Rust file: {}", path.display()),
            };
        };

        let template_id = path.to_string_lossy().into_owned();
        let outcome = diff_rsx_source(&template_id, old_source, new_source, generation);
        if matches!(outcome, DiffOutcome::NoOp | DiffOutcome::Tier1Patch(_)) {
            self.files.insert(key, new_source.to_owned());
        }
        outcome
    }
}

fn normalize_cache_key(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TokenKind<'a> {
    Ident(&'a str),
    Punct(char),
    StringLit { raw: &'a str, is_raw: bool },
    OtherLit(&'a str),
}

#[derive(Debug, Clone)]
struct LexResult<'a> {
    tokens: Vec<TokenKind<'a>>,
    /// For every `Punct('{')`, `Punct('(')`, or `Punct('[')` at index `i`, stores the index of its
    /// matching closing delimiter (`0` for non-opening tokens).
    matching_close: Vec<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum LexError {
    UnterminatedString,
    UnterminatedBlockComment,
    UnterminatedChar,
    UnbalancedDelimiter,
}

impl LexError {
    fn as_str(&self) -> &'static str {
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
fn tokenize(src: &str) -> Result<LexResult<'_>, LexError> {
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
            let ch = src[pos..].chars().next().unwrap();
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
            if pos + 1 < len && bytes[pos + 1] == b'r' {
                if let Some(end) = try_scan_raw_string(bytes, pos + 1)? {
                    tokens.push(TokenKind::OtherLit(&src[pos..end]));
                    matching_close.push(0);
                    pos = end;
                    continue;
                }
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
                if c.is_ascii_alphanumeric() || c == b'_' {
                    pos += 1;
                } else if c == b'.'
                    && pos + 1 < len
                    && bytes[pos + 1] != b'.'
                    && !is_ident_start(bytes[pos + 1])
                {
                    pos += 1;
                } else if (c == b'+' || c == b'-')
                    && pos > start
                    && (bytes[pos - 1] == b'e' || bytes[pos - 1] == b'E')
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
            let ch = src[pos..].chars().next().unwrap();
            pos += ch.len_utf8();
            while pos < len && is_ident_continue(bytes[pos]) {
                let c = src[pos..].chars().next().unwrap();
                pos += c.len_utf8();
            }
            tokens.push(TokenKind::Ident(&src[start..pos]));
            matching_close.push(0);
            continue;
        }

        // 10. Punctuation + delimiter balancing.
        let ch = src[pos..].chars().next().unwrap();
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
    let ch = src[pos..].chars().next().unwrap();
    let after_ch = pos + ch.len_utf8();
    if after_ch < len && bytes[after_ch] == b'\'' {
        return Ok(Some(after_ch + 1));
    }
    Ok(None)
}

/// Unescapes a Rust normal (`"..."`) or raw (`r#"..."#`) string literal into its string value.
fn unescape_string_literal(raw: &str, is_raw: bool) -> Result<String, LexError> {
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
fn has_interpolation_braces(s: &str) -> bool {
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

/// Maps a patchable Dioxus RSX style/layout attribute name to its canonical NVE/CSS attribute name.
/// Returns `None` for non-style attributes (`test_id`, `key`, event handlers, unknown props) so
/// edits to identity/logic props fall back safely to Tier-2.
fn normalize_patchable_attr_name(name: &str) -> Option<String> {
    match name {
        "style" => Some("style".into()),
        "width" | "height" | "min_width" | "min-width" | "min_height" | "min-height"
        | "max_width" | "max-width" | "max_height" | "max-height" | "flex_direction"
        | "flex-direction" | "justify_content" | "justify-content" | "align_items"
        | "align-items" | "flex" | "flex_grow" | "flex-grow" | "flex_shrink" | "flex-shrink"
        | "gap" | "padding" | "padding_horizontal" | "padding-horizontal" | "padding_vertical"
        | "padding-vertical" | "margin" | "background" | "background_color"
        | "background-color" | "color" | "border_color" | "border-color" | "border_width"
        | "border-width" | "border_radius" | "border-radius" | "opacity" | "font_size"
        | "font-size" | "z_index" | "z-index" => Some(name.replace('_', "-")),
        _ => None,
    }
}

fn is_test_id_attr(name: &str) -> bool {
    matches!(name, "test_id" | "testID" | "data-testid" | "test-id")
}

fn is_rust_control_keyword(ident: &str) -> bool {
    matches!(
        ident,
        "if" | "else"
            | "for"
            | "match"
            | "while"
            | "loop"
            | "let"
            | "move"
            | "async"
            | "unsafe"
            | "return"
            | "fn"
            | "struct"
            | "enum"
            | "impl"
            | "mod"
            | "use"
    )
}

#[derive(Debug, Clone)]
enum RsxLitContext {
    Attr {
        /// Token index of the owning RSX element's opening `{`, uniquely identifying the element.
        element_open_idx: usize,
        attr_name: String,
        element_test_id: Option<String>,
        node_path: Vec<usize>,
    },
    Text {
        text_index: usize,
        parent_test_id: Option<String>,
        node_path: Vec<usize>,
    },
}

struct RsxIndex {
    literals: HashMap<usize, RsxLitContext>,
    next_text_index: usize,
}

impl RsxIndex {
    fn build(lex: &LexResult<'_>) -> Self {
        let mut idx = Self {
            literals: HashMap::new(),
            next_text_index: 0,
        };
        idx.scan_rust_range(lex, 0, lex.tokens.len());
        idx
    }

    /// Scans normal Rust tokens looking for `rsx! { ... }` macro invocations.
    fn scan_rust_range(&mut self, lex: &LexResult<'_>, start: usize, end: usize) {
        let mut pos = start;
        while pos + 2 < end {
            if matches!(lex.tokens[pos], TokenKind::Ident("rsx"))
                && matches!(lex.tokens[pos + 1], TokenKind::Punct('!'))
                && matches!(lex.tokens[pos + 2], TokenKind::Punct('{'))
            {
                let open = pos + 2;
                let close = lex.matching_close[open];
                if close > open && close < end {
                    let mut path = Vec::new();
                    self.scan_rsx_children(lex, open + 1, close, &mut path, None);
                    pos = close + 1;
                    continue;
                }
            }
            pos += 1;
        }
    }

    fn scan_rsx_children(
        &mut self,
        lex: &LexResult<'_>,
        start: usize,
        end: usize,
        parent_path: &mut Vec<usize>,
        parent_test_id: Option<&str>,
    ) {
        let mut pos = start;
        let mut child_idx = 0usize;
        while pos < end {
            pos = self.scan_one_rsx_child(
                lex,
                pos,
                end,
                parent_path,
                parent_test_id,
                &mut child_idx,
            );
        }
    }

    /// Scans one RSX child item starting at `pos` and returns the next token index.
    fn scan_one_rsx_child(
        &mut self,
        lex: &LexResult<'_>,
        pos: usize,
        end: usize,
        parent_path: &mut Vec<usize>,
        parent_test_id: Option<&str>,
        child_idx: &mut usize,
    ) -> usize {
        match lex.tokens[pos] {
            TokenKind::Punct(',') | TokenKind::Punct(';') => pos + 1,

            // `if <cond> { <rsx> } (else if <cond> { <rsx> })* (else { <rsx> })?`
            TokenKind::Ident("if") => {
                let mut cur = pos + 1;
                loop {
                    let Some(brace_open) = find_block_brace_at_depth0(lex, cur, end) else {
                        self.scan_rust_range(lex, cur, end);
                        return end;
                    };
                    self.scan_rust_range(lex, cur, brace_open);
                    let brace_close = lex.matching_close[brace_open];
                    self.scan_rsx_children(
                        lex,
                        brace_open + 1,
                        brace_close,
                        parent_path,
                        parent_test_id,
                    );
                    cur = brace_close + 1;
                    if cur < end && matches!(lex.tokens[cur], TokenKind::Ident("else")) {
                        cur += 1;
                        if cur < end && matches!(lex.tokens[cur], TokenKind::Ident("if")) {
                            cur += 1;
                            continue;
                        } else if cur < end && matches!(lex.tokens[cur], TokenKind::Punct('{')) {
                            let else_close = lex.matching_close[cur];
                            self.scan_rsx_children(
                                lex,
                                cur + 1,
                                else_close,
                                parent_path,
                                parent_test_id,
                            );
                            return else_close + 1;
                        }
                    }
                    return cur;
                }
            }

            // `for <pat> in <iter> { <rsx> }`
            TokenKind::Ident("for") => {
                let Some(brace_open) = find_block_brace_at_depth0(lex, pos + 1, end) else {
                    self.scan_rust_range(lex, pos + 1, end);
                    return end;
                };
                self.scan_rust_range(lex, pos + 1, brace_open);
                let brace_close = lex.matching_close[brace_open];
                self.scan_rsx_children(
                    lex,
                    brace_open + 1,
                    brace_close,
                    parent_path,
                    parent_test_id,
                );
                brace_close + 1
            }

            // `match <expr> { ... }` inside RSX: treat body as Rust code (still finds nested `rsx!`).
            TokenKind::Ident("match") => {
                if let Some(brace_open) = find_block_brace_at_depth0(lex, pos + 1, end) {
                    let brace_close = lex.matching_close[brace_open];
                    self.scan_rust_range(lex, pos + 1, brace_close);
                    brace_close + 1
                } else {
                    end
                }
            }

            // Embedded Rust block `{ ... }` inside RSX (may contain nested `rsx! { ... }`).
            TokenKind::Punct('{') => {
                let close = lex.matching_close[pos];
                self.scan_rust_range(lex, pos + 1, close);
                close + 1
            }

            // RSX element or component: `tag { ... }`
            TokenKind::Ident(tag)
                if !is_rust_control_keyword(tag)
                    && pos + 1 < end
                    && matches!(lex.tokens[pos + 1], TokenKind::Punct('{')) =>
            {
                let open = pos + 1;
                let close = lex.matching_close[open];
                let idx = *child_idx;
                *child_idx += 1;
                parent_path.push(idx);
                self.scan_rsx_element(lex, open, close, parent_path);
                parent_path.pop();
                close + 1
            }

            // Static RSX text literal child (`"hello"`, `r#"..."#`).
            TokenKind::StringLit { .. }
                if !(pos + 1 < end && matches!(lex.tokens[pos + 1], TokenKind::Punct(':'))) =>
            {
                let idx = *child_idx;
                *child_idx += 1;
                let text_index = self.next_text_index;
                self.next_text_index += 1;
                let mut text_path = parent_path.clone();
                text_path.push(idx);
                self.literals.insert(
                    pos,
                    RsxLitContext::Text {
                        text_index,
                        parent_test_id: parent_test_id.map(ToOwned::to_owned),
                        node_path: text_path,
                    },
                );
                pos + 1
            }

            // Skip any other token (e.g. shorthand prop or path segment) safely.
            TokenKind::Punct('(') | TokenKind::Punct('[') => {
                let close = lex.matching_close[pos];
                self.scan_rust_range(lex, pos + 1, close);
                close + 1
            }
            _ => pos + 1,
        }
    }

    fn scan_rsx_element(
        &mut self,
        lex: &LexResult<'_>,
        open: usize,
        close: usize,
        elem_path: &mut Vec<usize>,
    ) {
        // Pass 1: discover static `test_id` / `"data-testid"` / `"testID"` on this element so
        // attributes declared before `test_id` still carry the element's `target_test_id`.
        let elem_test_id = find_element_static_test_id(lex, open + 1, close);

        // Pass 2: walk attributes and child items inside `{ ... }`.
        let mut pos = open + 1;
        let mut child_idx = 0usize;
        while pos < close {
            if let Some((attr_name, val_start, val_end, next_pos)) =
                parse_rsx_attribute(lex, pos, close)
            {
                if val_end == val_start + 1 {
                    if let TokenKind::StringLit { .. } = lex.tokens[val_start] {
                        if let Some(css_attr) = normalize_patchable_attr_name(&attr_name) {
                            self.literals.insert(
                                val_start,
                                RsxLitContext::Attr {
                                    element_open_idx: open,
                                    attr_name: css_attr,
                                    element_test_id: elem_test_id.clone(),
                                    node_path: elem_path.clone(),
                                },
                            );
                        }
                    }
                } else {
                    self.scan_rust_range(lex, val_start, val_end);
                }
                pos = next_pos;
                continue;
            }

            // Shorthand component prop (`accent,`) — identifier followed by `,` (not an element or text).
            if matches!(lex.tokens[pos], TokenKind::Ident(id) if !is_rust_control_keyword(id))
                && pos + 1 < close
                && matches!(lex.tokens[pos + 1], TokenKind::Punct(','))
            {
                pos += 2;
                continue;
            }

            pos = self.scan_one_rsx_child(
                lex,
                pos,
                close,
                elem_path,
                elem_test_id.as_deref(),
                &mut child_idx,
            );
        }
    }
}

/// Scans an element's depth-0 attributes for a static `test_id` / `testID` / `"data-testid"`.
fn find_element_static_test_id(lex: &LexResult<'_>, start: usize, end: usize) -> Option<String> {
    let mut pos = start;
    while pos < end {
        if let Some((attr_name, val_start, val_end, next_pos)) = parse_rsx_attribute(lex, pos, end)
        {
            if is_test_id_attr(&attr_name) && val_end == val_start + 1 {
                if let TokenKind::StringLit { raw, is_raw } = lex.tokens[val_start] {
                    if let Ok(val) = unescape_string_literal(raw, is_raw) {
                        if !val.is_empty() && !has_interpolation_braces(&val) {
                            return Some(val);
                        }
                    }
                }
            }
            pos = next_pos;
            continue;
        }
        match lex.tokens[pos] {
            TokenKind::Punct('{') | TokenKind::Punct('(') | TokenKind::Punct('[') => {
                pos = lex.matching_close[pos] + 1;
            }
            _ => pos += 1,
        }
    }
    None
}

/// Parses an RSX attribute `name: <expr> [,|}]` or `"quoted-name": <expr> [,|}]` at `pos`.
/// Returns `(attr_name, val_start, val_end, next_pos_after_comma)`.
fn parse_rsx_attribute(
    lex: &LexResult<'_>,
    pos: usize,
    end: usize,
) -> Option<(String, usize, usize, usize)> {
    if pos + 2 >= end {
        return None;
    }
    if !matches!(lex.tokens[pos + 1], TokenKind::Punct(':')) {
        return None;
    }
    // Reject `::` Rust paths (e.g., `std::foo`).
    if matches!(lex.tokens[pos + 2], TokenKind::Punct(':')) {
        return None;
    }
    let attr_name = match lex.tokens[pos] {
        TokenKind::Ident(id) if !is_rust_control_keyword(id) => id.to_owned(),
        TokenKind::StringLit { raw, is_raw } => unescape_string_literal(raw, is_raw).ok()?,
        _ => return None,
    };

    let val_start = pos + 2;
    let mut cur = val_start;
    while cur < end {
        match lex.tokens[cur] {
            TokenKind::Punct(',') => {
                return Some((attr_name, val_start, cur, cur + 1));
            }
            TokenKind::Punct('{') | TokenKind::Punct('(') | TokenKind::Punct('[') => {
                cur = lex.matching_close[cur] + 1;
            }
            _ => cur += 1,
        }
    }
    Some((attr_name, val_start, end, end))
}

/// Finds the opening `{` of an `if` / `for` / `match` block at depth 0, skipping balanced `()`/`[]`.
fn find_block_brace_at_depth0(lex: &LexResult<'_>, start: usize, end: usize) -> Option<usize> {
    let mut cur = start;
    while cur < end {
        match lex.tokens[cur] {
            TokenKind::Punct('{') => return Some(cur),
            TokenKind::Punct('(') | TokenKind::Punct('[') => {
                cur = lex.matching_close[cur] + 1;
            }
            _ => cur += 1,
        }
    }
    None
}

/// Diffs `old_src` vs `new_src` for a single `.rs` file and classifies the edit as `NoOp`,
/// `Tier1Patch`, or `RequireTier2Rebuild`.
pub fn diff_rsx_source(
    template_id: &str,
    old_src: &str,
    new_src: &str,
    generation: u64,
) -> DiffOutcome {
    if old_src == new_src {
        return DiffOutcome::NoOp;
    }

    let old_lex = match tokenize(old_src) {
        Ok(l) => l,
        Err(e) => {
            return DiffOutcome::RequireTier2Rebuild {
                reason: format!("baseline lex error: {}", e.as_str()),
            };
        }
    };
    let new_lex = match tokenize(new_src) {
        Ok(l) => l,
        Err(e) => {
            return DiffOutcome::RequireTier2Rebuild {
                reason: format!("syntax error: {}", e.as_str()),
            };
        }
    };

    if old_lex.tokens.len() != new_lex.tokens.len() {
        return DiffOutcome::RequireTier2Rebuild {
            reason: "token count changed (structural or logic edit)".into(),
        };
    }

    let mut diff_indices = Vec::new();
    for i in 0..old_lex.tokens.len() {
        if old_lex.tokens[i] == new_lex.tokens[i] {
            continue;
        }
        match (&old_lex.tokens[i], &new_lex.tokens[i]) {
            (TokenKind::StringLit { .. }, TokenKind::StringLit { .. }) => {
                diff_indices.push(i);
            }
            _ => {
                return DiffOutcome::RequireTier2Rebuild {
                    reason: "non-string Rust token changed".into(),
                };
            }
        }
    }

    if diff_indices.is_empty() {
        return DiffOutcome::NoOp;
    }

    // Only string literals changed; verify EVERY changed literal lives inside `rsx! { ... }`
    // as a pure static (non-interpolated) text child or patchable style/layout attribute.
    let rsx_index = RsxIndex::build(&old_lex);

    let mut attr_updates: Vec<(String, String)> = Vec::new();
    let mut text_updates: Vec<(usize, String)> = Vec::new();
    let mut text_replacements: Vec<(String, String)> = Vec::new();
    let mut target_test_id: Option<String> = None;
    let mut node_path: Vec<usize> = Vec::new();
    let mut attr_owner_elem: Option<usize> = None;
    let mut text_target_test_id: Option<String> = None;
    let mut text_target_path: Vec<usize> = Vec::new();
    let mut text_owners_uniform = true;
    let mut seen_text_change = false;

    for idx in diff_indices {
        let (old_raw, old_is_raw) = match old_lex.tokens[idx] {
            TokenKind::StringLit { raw, is_raw } => (raw, is_raw),
            _ => unreachable!(),
        };
        let (new_raw, new_is_raw) = match new_lex.tokens[idx] {
            TokenKind::StringLit { raw, is_raw } => (raw, is_raw),
            _ => unreachable!(),
        };

        let old_val = match unescape_string_literal(old_raw, old_is_raw) {
            Ok(v) => v,
            Err(e) => {
                return DiffOutcome::RequireTier2Rebuild {
                    reason: format!("invalid old string literal: {}", e.as_str()),
                };
            }
        };
        let new_val = match unescape_string_literal(new_raw, new_is_raw) {
            Ok(v) => v,
            Err(e) => {
                return DiffOutcome::RequireTier2Rebuild {
                    reason: format!("invalid new string literal: {}", e.as_str()),
                };
            }
        };

        // Semantically identical after raw/escaped normalization (e.g., `"hi"` vs `r#"hi"#`).
        if old_val == new_val {
            continue;
        }

        // Any format-string `{expr}` interpolation brace requires Rust recompilation.
        if has_interpolation_braces(&old_val) || has_interpolation_braces(&new_val) {
            return DiffOutcome::RequireTier2Rebuild {
                reason: "interpolated `{expr}` string literal changed".into(),
            };
        }

        let Some(ctx) = rsx_index.literals.get(&idx) else {
            return DiffOutcome::RequireTier2Rebuild {
                reason: "string literal outside patchable rsx! text/style context changed".into(),
            };
        };

        match ctx {
            RsxLitContext::Attr {
                element_open_idx,
                attr_name,
                element_test_id,
                node_path: elem_path,
            } => {
                if let Some(prev_elem) = attr_owner_elem {
                    if prev_elem != *element_open_idx {
                        return DiffOutcome::RequireTier2Rebuild {
                            reason: "attribute edits span multiple RSX elements".into(),
                        };
                    }
                } else {
                    attr_owner_elem = Some(*element_open_idx);
                    target_test_id = element_test_id.clone();
                    node_path = elem_path.clone();
                }
                attr_updates.push((attr_name.clone(), new_val));
            }
            RsxLitContext::Text {
                text_index,
                parent_test_id,
                node_path: tpath,
            } => {
                if old_val.is_empty() {
                    return DiffOutcome::RequireTier2Rebuild {
                        reason: "empty static text literal cannot be matched uniquely".into(),
                    };
                }
                if !seen_text_change {
                    seen_text_change = true;
                    text_target_test_id = parent_test_id.clone();
                    text_target_path = tpath.clone();
                } else if text_target_test_id != *parent_test_id {
                    text_owners_uniform = false;
                }
                text_replacements.push((old_val, new_val.clone()));
                text_updates.push((*text_index, new_val));
            }
        }
    }

    if attr_updates.is_empty() && text_replacements.is_empty() {
        return DiffOutcome::NoOp;
    }

    // If no attribute edit claimed targeting, carry the single text element's targeting metadata.
    if attr_owner_elem.is_none() && seen_text_change {
        if text_owners_uniform {
            target_test_id = text_target_test_id;
        }
        if text_replacements.len() == 1 {
            node_path = text_target_path;
        }
    }

    DiffOutcome::Tier1Patch(HotPatchMsg::TemplatePatch {
        generation,
        template_id: template_id.to_owned(),
        attr_updates,
        text_updates,
        target_test_id,
        node_path,
        text_replacements,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    const SAMPLE_COMPONENT: &str = r##"
use nve_dioxus::prelude::*;

#[component]
pub fn DemoCard() -> Element {
    let mut count = use_signal(|| 0i32);
    rsx! {
        view {
            test_id: "card-root",
            style: "width: 100%; padding: 12px; background-color: #0f172a;",
            text {
                test_id: "channel-title",
                style: "color: #ffffff; font-size: 16px;",
                "# engineering"
            }
            pressable {
                test_id: "cta-btn",
                background_color: "#2563eb",
                onpress: move |_| count += 1,
                text { "Count: {count}" }
                text { "Ship It" }
            }
        }
    }
}
"##;

    #[test]
    fn tier1_text_only_edit_inside_rsx_emits_targeted_replacement() {
        let updated = SAMPLE_COMPONENT.replace("\"# engineering\"", "\"# ergoiherg\"");
        let outcome = diff_rsx_source("demo.rs", SAMPLE_COMPONENT, &updated, 2);
        match outcome {
            DiffOutcome::Tier1Patch(HotPatchMsg::TemplatePatch {
                generation,
                template_id,
                attr_updates,
                text_updates,
                target_test_id,
                node_path,
                text_replacements,
            }) => {
                assert_eq!(generation, 2);
                assert_eq!(template_id, "demo.rs");
                assert!(attr_updates.is_empty());
                assert_eq!(
                    text_replacements,
                    vec![("# engineering".into(), "# ergoiherg".into())]
                );
                assert_eq!(text_updates, vec![(0, "# ergoiherg".into())]);
                assert_eq!(target_test_id.as_deref(), Some("channel-title"));
                assert_eq!(node_path, vec![0, 0, 0]);
            }
            other => panic!("expected Tier1Patch, got {other:?}"),
        }
    }

    #[test]
    fn tier1_style_and_attr_literal_edits_target_exact_test_id() {
        let updated = SAMPLE_COMPONENT.replace(
            "width: 100%; padding: 12px; background-color: #0f172a;",
            "width: 100%; padding: 24px; background-color: #1e293b;",
        );
        match diff_rsx_source("demo.rs", SAMPLE_COMPONENT, &updated, 3) {
            DiffOutcome::Tier1Patch(HotPatchMsg::TemplatePatch {
                attr_updates,
                text_replacements,
                target_test_id,
                node_path,
                ..
            }) => {
                assert_eq!(
                    attr_updates,
                    vec![(
                        "style".into(),
                        "width: 100%; padding: 24px; background-color: #1e293b;".into()
                    )]
                );
                assert!(text_replacements.is_empty());
                assert_eq!(target_test_id.as_deref(), Some("card-root"));
                assert_eq!(node_path, vec![0]);
            }
            other => panic!("expected Tier1Patch, got {other:?}"),
        }

        // Direct typed attribute (`background_color` -> normalized `background-color`).
        let updated_attr = SAMPLE_COMPONENT.replace("\"#2563eb\"", "\"#16a34a\"");
        match diff_rsx_source("demo.rs", SAMPLE_COMPONENT, &updated_attr, 4) {
            DiffOutcome::Tier1Patch(HotPatchMsg::TemplatePatch {
                attr_updates,
                target_test_id,
                ..
            }) => {
                assert_eq!(
                    attr_updates,
                    vec![("background-color".into(), "#16a34a".into())]
                );
                assert_eq!(target_test_id.as_deref(), Some("cta-btn"));
            }
            other => panic!("expected Tier1Patch for background_color, got {other:?}"),
        }
    }

    #[test]
    fn tier1_multiple_edits_in_one_save() {
        // Edit both a style on `channel-title` and two static text literals in one save.
        let updated = SAMPLE_COMPONENT
            .replace("color: #ffffff; font-size: 16px;", "color: #38bdf8; font-size: 18px;")
            .replace("\"# engineering\"", "\"# platform\"")
            .replace("\"Ship It\"", "\"Deploy Now\"");

        match diff_rsx_source("demo.rs", SAMPLE_COMPONENT, &updated, 5) {
            DiffOutcome::Tier1Patch(HotPatchMsg::TemplatePatch {
                attr_updates,
                text_replacements,
                text_updates,
                target_test_id,
                ..
            }) => {
                assert_eq!(
                    attr_updates,
                    vec![("style".into(), "color: #38bdf8; font-size: 18px;".into())]
                );
                assert_eq!(target_test_id.as_deref(), Some("channel-title"));
                assert_eq!(
                    text_replacements,
                    vec![
                        ("# engineering".into(), "# platform".into()),
                        ("Ship It".into(), "Deploy Now".into()),
                    ]
                );
                assert_eq!(text_updates.len(), 2);
            }
            other => panic!("expected Tier1Patch, got {other:?}"),
        }
    }

    #[test]
    fn interpolated_literal_structural_logic_handler_and_syntax_edits_fallback_to_tier2() {
        // 1. Format `{expr}` interpolation literal changed -> Tier-2.
        let interp = SAMPLE_COMPONENT.replace("\"Count: {count}\"", "\"Total: {count}\"");
        assert!(matches!(
            diff_rsx_source("demo.rs", SAMPLE_COMPONENT, &interp, 2),
            DiffOutcome::RequireTier2Rebuild { .. }
        ));

        // 2. Adding interpolation braces to a previously static literal -> Tier-2.
        let add_interp = SAMPLE_COMPONENT.replace("\"Ship It\"", "\"Ship {count}\"");
        assert!(matches!(
            diff_rsx_source("demo.rs", SAMPLE_COMPONENT, &add_interp, 2),
            DiffOutcome::RequireTier2Rebuild { .. }
        ));

        // 3. Event handler logic changed -> Tier-2.
        let handler = SAMPLE_COMPONENT.replace("count += 1", "count += 2");
        assert!(matches!(
            diff_rsx_source("demo.rs", SAMPLE_COMPONENT, &handler, 2),
            DiffOutcome::RequireTier2Rebuild { .. }
        ));

        // 4. Structural edit (adding a new element) -> Tier-2.
        let structural =
            SAMPLE_COMPONENT.replace("text { \"Ship It\" }", "text { \"Ship It\" } text { \"New\" }");
        assert!(matches!(
            diff_rsx_source("demo.rs", SAMPLE_COMPONENT, &structural, 2),
            DiffOutcome::RequireTier2Rebuild { .. }
        ));

        // 5. Editing `test_id` itself -> Tier-2.
        let tid_edit = SAMPLE_COMPONENT.replace("test_id: \"cta-btn\"", "test_id: \"cta-btn-2\"");
        assert!(matches!(
            diff_rsx_source("demo.rs", SAMPLE_COMPONENT, &tid_edit, 2),
            DiffOutcome::RequireTier2Rebuild { .. }
        ));

        // 6. Syntax error (unclosed string / unbalanced brace) -> Tier-2.
        let unclosed_str = SAMPLE_COMPONENT.replace("\"Ship It\"", "\"Ship It");
        assert!(matches!(
            diff_rsx_source("demo.rs", SAMPLE_COMPONENT, &unclosed_str, 2),
            DiffOutcome::RequireTier2Rebuild { .. }
        ));
        let unbalanced_brace = format!("{SAMPLE_COMPONENT}\n}}");
        assert!(matches!(
            diff_rsx_source("demo.rs", SAMPLE_COMPONENT, &unbalanced_brace, 2),
            DiffOutcome::RequireTier2Rebuild { .. }
        ));

        // 7. String literal outside `rsx!` changed -> Tier-2.
        let with_rust_str = format!("const LABEL: &str = \"hello\";\n{SAMPLE_COMPONENT}");
        let with_rust_str_edited = format!("const LABEL: &str = \"world\";\n{SAMPLE_COMPONENT}");
        assert!(matches!(
            diff_rsx_source("demo.rs", &with_rust_str, &with_rust_str_edited, 2),
            DiffOutcome::RequireTier2Rebuild { .. }
        ));

        // 8. Attribute edits spanning two different elements -> Tier-2 (never mis-target).
        let two_elems = SAMPLE_COMPONENT
            .replace("padding: 12px;", "padding: 20px;")
            .replace("\"#2563eb\"", "\"#ef4444\"");
        assert!(matches!(
            diff_rsx_source("demo.rs", SAMPLE_COMPONENT, &two_elems, 2),
            DiffOutcome::RequireTier2Rebuild { .. }
        ));
    }

    #[test]
    fn comment_and_whitespace_only_edits_are_noop() {
        let commented = format!(
            "// top-level comment\n/* outer /* nested */ comment */\n{}\n   ",
            SAMPLE_COMPONENT.replace("rsx! {", "rsx! { // inline comment\n")
        );
        assert_eq!(
            diff_rsx_source("demo.rs", SAMPLE_COMPONENT, &commented, 2),
            DiffOutcome::NoOp
        );
    }

    #[test]
    fn watch_cache_chains_consecutive_tier1_patches_and_protects_baseline_on_error() {
        let mut cache = RsxWatchCache::new();
        let path = PathBuf::from("src/demo.rs");
        cache.update_file(&path, SAMPLE_COMPONENT.to_owned());

        // First Tier-1 edit: "# engineering" -> "# core"
        let v1 = SAMPLE_COMPONENT.replace("\"# engineering\"", "\"# core\"");
        match cache.diff_file(&path, &v1, 2) {
            DiffOutcome::Tier1Patch(HotPatchMsg::TemplatePatch {
                text_replacements, ..
            }) => {
                assert_eq!(
                    text_replacements,
                    vec![("# engineering".into(), "# core".into())]
                );
            }
            other => panic!("expected Tier1Patch v1, got {other:?}"),
        }

        // Second consecutive Tier-1 edit chains cleanly from "# core" -> "# runtime"!
        let v2 = v1.replace("\"# core\"", "\"# runtime\"");
        match cache.diff_file(&path, &v2, 3) {
            DiffOutcome::Tier1Patch(HotPatchMsg::TemplatePatch {
                text_replacements, ..
            }) => {
                assert_eq!(text_replacements, vec![("# core".into(), "# runtime".into())]);
            }
            other => panic!("expected Tier1Patch v2, got {other:?}"),
        }

        // Broken syntax edit triggers Tier-2 and preserves `v2` as baseline.
        let broken = v2.replace("\"# runtime\"", "\"# runtime");
        assert!(matches!(
            cache.diff_file(&path, &broken, 4),
            DiffOutcome::RequireTier2Rebuild { .. }
        ));
        assert_eq!(cache.cached_source(&path), Some(v2.as_str()));
    }

    #[test]
    fn diff_latency_on_real_studio_workspace_ui_rs_is_under_10ms() {
        let ui_path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/studio-workspace/src/ui.rs");
        let original = std::fs::read_to_string(&ui_path)
            .unwrap_or_else(|e| panic!("failed reading {}: {e}", ui_path.display()));

        // 1. Static text edit inside `rsx!` ("QUICK REPLY" -> "FAST REPLY")
        let edited_text = original.replacen("\"QUICK REPLY\"", "\"FAST REPLY\"", 1);
        assert_ne!(original, edited_text);

        let t0 = Instant::now();
        let outcome = diff_rsx_source(
            "examples/studio-workspace/src/ui.rs",
            &original,
            &edited_text,
            2,
        );
        let elapsed = t0.elapsed();

        match outcome {
            DiffOutcome::Tier1Patch(HotPatchMsg::TemplatePatch {
                text_replacements,
                attr_updates,
                ..
            }) => {
                assert!(attr_updates.is_empty());
                assert_eq!(
                    text_replacements,
                    vec![("QUICK REPLY".into(), "FAST REPLY".into())]
                );
            }
            other => panic!("expected Tier1Patch on studio-workspace ui.rs, got {other:?}"),
        }
        assert!(
            elapsed.as_millis() < 10,
            "expected Tier-1 diff on real ui.rs ({} bytes) in < 10ms, took {:?}",
            original.len(),
            elapsed
        );
        eprintln!(
            "[bench] Tier-1 text diff on examples/studio-workspace/src/ui.rs ({} bytes): {:.3} ms ({} us)",
            original.len(),
            elapsed.as_secs_f64() * 1000.0,
            elapsed.as_micros()
        );

        // 2. Static style edit inside `rsx!` (`style: "width: 100%; flex: 1;"`)
        let edited_style = original.replacen(
            "style: \"width: 100%; flex: 1;\"",
            "style: \"width: 100%; flex: 1; padding: 8px;\"",
            1,
        );
        assert_ne!(original, edited_style);

        let t1 = Instant::now();
        let style_outcome = diff_rsx_source(
            "examples/studio-workspace/src/ui.rs",
            &original,
            &edited_style,
            3,
        );
        let style_elapsed = t1.elapsed();
        match style_outcome {
            DiffOutcome::Tier1Patch(HotPatchMsg::TemplatePatch { attr_updates, .. }) => {
                assert_eq!(
                    attr_updates,
                    vec![("style".into(), "width: 100%; flex: 1; padding: 8px;".into())]
                );
            }
            other => panic!("expected Tier1Patch for style edit on ui.rs, got {other:?}"),
        }
        assert!(
            style_elapsed.as_millis() < 10,
            "expected Tier-1 style diff on ui.rs in < 10ms, took {:?}",
            style_elapsed
        );
        eprintln!(
            "[bench] Tier-1 style diff on examples/studio-workspace/src/ui.rs ({} bytes): {:.3} ms ({} us)",
            original.len(),
            style_elapsed.as_secs_f64() * 1000.0,
            style_elapsed.as_micros()
        );

        // 3. Rust logic / tuple data edit outside `rsx!` falls back cleanly to Tier-2.
        let edited_outside = original.replacen("\"# engineering\"", "\"# ergoiherg\"", 1);
        assert!(matches!(
            diff_rsx_source(
                "examples/studio-workspace/src/ui.rs",
                &original,
                &edited_outside,
                4
            ),
            DiffOutcome::RequireTier2Rebuild { .. }
        ));
    }
}
