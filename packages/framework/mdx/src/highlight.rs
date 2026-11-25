//! Zero-dependency pure-Rust compile-time syntax highlighter.

pub use crate::highlight_langs::Language;
use crate::highlight_scan::{
    advance_ws, detect_rust_raw_string, is_ident_start, is_pascal_type, scan_backtick,
    scan_block_comment, scan_dollar_var, scan_ident, scan_number, scan_quoted,
    scan_rust_attribute, scan_rust_raw_string, scan_triple_quote, try_scan_char_literal,
};

/// Semantic syntax token classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TokenKind {
    Plain,
    Keyword,
    TypeName,
    StringLit,
    Comment,
    Number,
    Function,
    Macro,
    Attribute,
    Punctuation,
    DiffAdd,
    DiffDel,
}

impl TokenKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Plain => "plain",
            Self::Keyword => "keyword",
            Self::TypeName => "type",
            Self::StringLit => "string",
            Self::Comment => "comment",
            Self::Number => "number",
            Self::Function => "function",
            Self::Macro => "macro",
            Self::Attribute => "attribute",
            Self::Punctuation => "punctuation",
            Self::DiffAdd => "diff-add",
            Self::DiffDel => "diff-del",
        }
    }
}

/// Single highlighted token span within a line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HighlightSpan {
    pub kind: TokenKind,
    pub text: String,
}

/// Single highlighted source line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HighlightLine {
    pub line_no: usize,
    pub highlighted: bool,
    pub tokens: Vec<HighlightSpan>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MultiLineState {
    None,
    BlockComment { depth: usize },
    TripleQuote { quote: u8 },
    BacktickString,
    RustRawString { hashes: usize },
}

/// Highlight `code` for `lang_tag`, flagging 1-based lines present in `highlight_lines`.
pub fn highlight_code(
    lang_tag: Option<&str>,
    code: &str,
    highlight_lines: &[usize],
) -> Vec<HighlightLine> {
    let lang = Language::from_tag(lang_tag);
    if code.is_empty() {
        return Vec::new();
    }

    let mut state = MultiLineState::None;
    let mut lines_out = Vec::new();

    for (idx, line) in code.split('\n').enumerate() {
        let line = line.strip_suffix('\r').unwrap_or(line);
        let line_no = idx + 1;
        let is_hl = highlight_lines.contains(&line_no);
        let tokens = match lang {
            Language::Plain => {
                if line.is_empty() {
                    Vec::new()
                } else {
                    vec![HighlightSpan {
                        kind: TokenKind::Plain,
                        text: line.to_string(),
                    }]
                }
            }
            Language::Diff => highlight_diff_line(line),
            Language::Toml => highlight_toml_line(line, &mut state),
            Language::Json => highlight_json_line(line),
            _ => highlight_generic_line(line, lang, &mut state),
        };
        lines_out.push(HighlightLine {
            line_no,
            highlighted: is_hl,
            tokens,
        });
    }

    lines_out
}

fn push_span(out: &mut Vec<HighlightSpan>, kind: TokenKind, text: &str) {
    if text.is_empty() {
        return;
    }
    if let Some(last) = out.last_mut()
        && last.kind == kind
    {
        last.text.push_str(text);
        return;
    }
    out.push(HighlightSpan {
        kind,
        text: text.to_string(),
    });
}

fn highlight_diff_line(line: &str) -> Vec<HighlightSpan> {
    if line.is_empty() {
        return Vec::new();
    }
    let kind = if line.starts_with("+++") || line.starts_with("---") || line.starts_with("diff ") {
        TokenKind::Comment
    } else if line.starts_with("@@") {
        TokenKind::Attribute
    } else if line.starts_with('+') {
        TokenKind::DiffAdd
    } else if line.starts_with('-') {
        TokenKind::DiffDel
    } else {
        TokenKind::Plain
    };
    vec![HighlightSpan {
        kind,
        text: line.to_string(),
    }]
}

fn highlight_toml_line(line: &str, state: &mut MultiLineState) -> Vec<HighlightSpan> {
    let mut out = Vec::new();
    let trimmed = line.trim_start();
    if *state == MultiLineState::None && trimmed.starts_with('[') && trimmed.trim_end().ends_with(']')
    {
        let lead = line.len() - trimmed.len();
        push_span(&mut out, TokenKind::Plain, &line[..lead]);
        push_span(&mut out, TokenKind::Attribute, trimmed);
        return out;
    }

    let bytes = line.as_bytes();
    let mut i = resume_multiline(line, state, Language::Toml, &mut out).unwrap_or(0);

    while i < bytes.len() {
        let rest = &line[i..];
        let b = bytes[i];
        if b == b'#' {
            push_span(&mut out, TokenKind::Comment, rest);
            break;
        }
        if b.is_ascii_whitespace() {
            let end = advance_ws(bytes, i);
            push_span(&mut out, TokenKind::Plain, &line[i..end]);
            i = end;
            continue;
        }
        if (rest.starts_with("\"\"\"") || rest.starts_with("'''")) && i + 3 <= bytes.len() {
            let (end, closed) = scan_triple_quote(line, i + 3, b);
            push_span(&mut out, TokenKind::StringLit, &line[i..end]);
            if !closed {
                *state = MultiLineState::TripleQuote { quote: b };
            }
            i = end;
            continue;
        }
        if b == b'"' || b == b'\'' {
            let end = scan_quoted(line, i, b, b == b'"');
            push_span(&mut out, TokenKind::StringLit, &line[i..end]);
            i = end;
            continue;
        }
        if b.is_ascii_digit() {
            let end = scan_number(bytes, i);
            push_span(&mut out, TokenKind::Number, &line[i..end]);
            i = end;
            continue;
        }
        if is_ident_start(b) {
            let end = scan_ident(bytes, i, true);
            let word = &line[i..end];
            let after = line[end..].trim_start();
            let kind = if Language::Toml.is_keyword(word) {
                TokenKind::Keyword
            } else if after.starts_with('=') || after.starts_with('.') {
                TokenKind::Attribute
            } else {
                TokenKind::Plain
            };
            push_span(&mut out, kind, word);
            i = end;
            continue;
        }
        let ch_len = rest.chars().next().map_or(1, char::len_utf8);
        let kind = if b.is_ascii_punctuation() {
            TokenKind::Punctuation
        } else {
            TokenKind::Plain
        };
        push_span(&mut out, kind, &line[i..i + ch_len]);
        i += ch_len;
    }
    out
}

fn highlight_json_line(line: &str) -> Vec<HighlightSpan> {
    let mut out = Vec::new();
    let bytes = line.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        let rest = &line[i..];
        let b = bytes[i];
        if rest.starts_with("//") {
            push_span(&mut out, TokenKind::Comment, rest);
            break;
        }
        if b.is_ascii_whitespace() {
            let end = advance_ws(bytes, i);
            push_span(&mut out, TokenKind::Plain, &line[i..end]);
            i = end;
            continue;
        }
        if b == b'"' {
            let end = scan_quoted(line, i, b'"', true);
            let kind = if line[end..].trim_start().starts_with(':') {
                TokenKind::Attribute
            } else {
                TokenKind::StringLit
            };
            push_span(&mut out, kind, &line[i..end]);
            i = end;
            continue;
        }
        if b.is_ascii_digit() || (b == b'-' && bytes.get(i + 1).is_some_and(u8::is_ascii_digit)) {
            let end = scan_number(bytes, i + usize::from(b == b'-'));
            push_span(&mut out, TokenKind::Number, &line[i..end]);
            i = end;
            continue;
        }
        if is_ident_start(b) {
            let end = scan_ident(bytes, i, false);
            let word = &line[i..end];
            let kind = if Language::Json.is_keyword(word) {
                TokenKind::Keyword
            } else {
                TokenKind::Plain
            };
            push_span(&mut out, kind, word);
            i = end;
            continue;
        }
        let ch_len = rest.chars().next().map_or(1, char::len_utf8);
        let kind = if b.is_ascii_punctuation() {
            TokenKind::Punctuation
        } else {
            TokenKind::Plain
        };
        push_span(&mut out, kind, &line[i..i + ch_len]);
        i += ch_len;
    }
    out
}

fn highlight_generic_line(
    line: &str,
    lang: Language,
    state: &mut MultiLineState,
) -> Vec<HighlightSpan> {
    let mut out = Vec::new();
    let bytes = line.as_bytes();
    let mut i = resume_multiline(line, state, lang, &mut out).unwrap_or(0);

    if lang == Language::Cpp && i == 0 {
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') {
            let lead = line.len() - trimmed.len();
            push_span(&mut out, TokenKind::Plain, &line[..lead]);
            let end_dir = scan_ident(trimmed.as_bytes(), 1, false);
            push_span(&mut out, TokenKind::Attribute, &trimmed[..end_dir]);
            let rem = &trimmed[end_dir..];
            if !rem.is_empty() {
                let mut sub_state = MultiLineState::None;
                for sp in highlight_generic_line(rem, Language::Cpp, &mut sub_state) {
                    push_span(&mut out, sp.kind, &sp.text);
                }
            }
            return out;
        }
    }

    while i < bytes.len() {
        let rest = &line[i..];
        let b = bytes[i];

        if b.is_ascii_whitespace() {
            let end = advance_ws(bytes, i);
            push_span(&mut out, TokenKind::Plain, &line[i..end]);
            i = end;
            continue;
        }
        if lang.supports_slash_comments() && rest.starts_with("//") {
            push_span(&mut out, TokenKind::Comment, rest);
            break;
        }
        if lang.supports_hash_comments() && b == b'#' {
            push_span(&mut out, TokenKind::Comment, rest);
            break;
        }
        if lang.supports_slash_comments() && rest.starts_with("/*") {
            let (end, depth) =
                scan_block_comment(line, i + 2, 1, lang.supports_nested_block_comments());
            push_span(&mut out, TokenKind::Comment, &line[i..end]);
            if depth > 0 {
                *state = MultiLineState::BlockComment { depth };
            }
            i = end;
            continue;
        }
        if lang == Language::Rust && (rest.starts_with("#[") || rest.starts_with("#![")) {
            let end = scan_rust_attribute(line, i);
            push_span(&mut out, TokenKind::Attribute, &line[i..end]);
            i = end;
            continue;
        }
        if lang == Language::Rust
            && let Some((prefix_len, hashes)) = detect_rust_raw_string(rest)
        {
            let (end, closed) = scan_rust_raw_string(line, i + prefix_len, hashes);
            push_span(&mut out, TokenKind::StringLit, &line[i..end]);
            if !closed {
                *state = MultiLineState::RustRawString { hashes };
            }
            i = end;
            continue;
        }
        if lang.supports_triple_quotes() && rest.starts_with("\"\"\"") {
            let (end, closed) = scan_triple_quote(line, i + 3, b'"');
            push_span(&mut out, TokenKind::StringLit, &line[i..end]);
            if !closed {
                *state = MultiLineState::TripleQuote { quote: b'"' };
            }
            i = end;
            continue;
        }
        if b == b'@' {
            if lang == Language::Cpp && bytes.get(i + 1) == Some(&b'"') {
                let end = scan_quoted(line, i + 1, b'"', true);
                push_span(&mut out, TokenKind::StringLit, &line[i..end]);
                i = end;
                continue;
            }
            if bytes.get(i + 1).is_some_and(|&c| is_ident_start(c)) {
                let end = scan_ident(bytes, i + 1, false);
                let kind = if lang == Language::Cpp {
                    TokenKind::Keyword
                } else {
                    TokenKind::Attribute
                };
                push_span(&mut out, kind, &line[i..end]);
                i = end;
                continue;
            }
        }
        if lang == Language::Swift
            && b == b'#'
            && bytes.get(i + 1).is_some_and(|&c| is_ident_start(c))
        {
            let end = scan_ident(bytes, i + 1, false);
            push_span(&mut out, TokenKind::Macro, &line[i..end]);
            i = end;
            continue;
        }
        if matches!(lang, Language::Shell | Language::Cmake) && b == b'$' {
            let end = scan_dollar_var(bytes, i);
            if end > i + 1 {
                push_span(&mut out, TokenKind::Macro, &line[i..end]);
                i = end;
                continue;
            }
        }
        if b == b'"' {
            let end = scan_quoted(line, i, b'"', true);
            push_span(&mut out, TokenKind::StringLit, &line[i..end]);
            i = end;
            continue;
        }
        if b == b'\'' {
            if lang.supports_single_quote_strings() {
                let end = scan_quoted(line, i, b'\'', lang != Language::Shell);
                push_span(&mut out, TokenKind::StringLit, &line[i..end]);
                i = end;
                continue;
            }
            if matches!(
                lang,
                Language::Rust | Language::Cpp | Language::Java | Language::Kotlin
            ) && let Some(end) = try_scan_char_literal(line, i)
            {
                push_span(&mut out, TokenKind::StringLit, &line[i..end]);
                i = end;
                continue;
            }
            if lang == Language::Rust && bytes.get(i + 1).is_some_and(|&c| is_ident_start(c)) {
                let end = scan_ident(bytes, i + 1, false);
                push_span(&mut out, TokenKind::TypeName, &line[i..end]);
                i = end;
                continue;
            }
        }
        if lang.supports_backtick_strings() && b == b'`' {
            let (end, closed) = scan_backtick(line, i + 1);
            push_span(&mut out, TokenKind::StringLit, &line[i..end]);
            if !closed && lang == Language::TypeScript {
                *state = MultiLineState::BacktickString;
            }
            i = end;
            continue;
        }
        if b.is_ascii_digit() {
            let end = scan_number(bytes, i);
            push_span(&mut out, TokenKind::Number, &line[i..end]);
            i = end;
            continue;
        }
        if is_ident_start(b) {
            let end = scan_ident(bytes, i, false);
            let word = &line[i..end];
            if lang == Language::Rust && bytes.get(end) == Some(&b'!') {
                push_span(&mut out, TokenKind::Macro, &line[i..end + 1]);
                i = end + 1;
                continue;
            }
            let after_ws = line[end..].trim_start();
            let kind = if lang.is_keyword(word) {
                TokenKind::Keyword
            } else if lang.is_builtin_type(word) {
                TokenKind::TypeName
            } else if after_ws.starts_with('(') {
                TokenKind::Function
            } else if is_pascal_type(word) && !matches!(lang, Language::Shell | Language::Cmake) {
                TokenKind::TypeName
            } else {
                TokenKind::Plain
            };
            push_span(&mut out, kind, word);
            i = end;
            continue;
        }

        let ch_len = rest.chars().next().map_or(1, char::len_utf8);
        let kind = if b.is_ascii_punctuation() {
            TokenKind::Punctuation
        } else {
            TokenKind::Plain
        };
        push_span(&mut out, kind, &line[i..i + ch_len]);
        i += ch_len;
    }

    out
}

fn resume_multiline(
    line: &str,
    state: &mut MultiLineState,
    lang: Language,
    out: &mut Vec<HighlightSpan>,
) -> Option<usize> {
    match *state {
        MultiLineState::None => None,
        MultiLineState::BlockComment { depth } => {
            let (end, rem_depth) =
                scan_block_comment(line, 0, depth, lang.supports_nested_block_comments());
            push_span(out, TokenKind::Comment, &line[..end]);
            *state = if rem_depth == 0 {
                MultiLineState::None
            } else {
                MultiLineState::BlockComment { depth: rem_depth }
            };
            Some(end)
        }
        MultiLineState::TripleQuote { quote } => {
            let (end, closed) = scan_triple_quote(line, 0, quote);
            push_span(out, TokenKind::StringLit, &line[..end]);
            if closed {
                *state = MultiLineState::None;
            }
            Some(end)
        }
        MultiLineState::BacktickString => {
            let (end, closed) = scan_backtick(line, 0);
            push_span(out, TokenKind::StringLit, &line[..end]);
            if closed {
                *state = MultiLineState::None;
            }
            Some(end)
        }
        MultiLineState::RustRawString { hashes } => {
            let (end, closed) = scan_rust_raw_string(line, 0, hashes);
            push_span(out, TokenKind::StringLit, &line[..end]);
            if closed {
                *state = MultiLineState::None;
            }
            Some(end)
        }
    }
}
