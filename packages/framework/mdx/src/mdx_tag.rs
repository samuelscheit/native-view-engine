//! Parsers for declarative `.mdx` `<Component ... />` tags and `:::{...}` directive attributes.

use std::collections::BTreeMap;

use crate::ast::{ComponentNode, ComponentProp, Frontmatter, PropValue, SourcePos};

/// Result of parsing an opening or self-closing MDX component tag from a string slice.
#[derive(Debug, Clone)]
pub(crate) struct ParsedOpenTag {
    pub name: String,
    pub props: Vec<ComponentProp>,
    pub self_closing: bool,
    pub pos: SourcePos,
    /// Byte index immediately after the closing `>` of the opening tag.
    pub end_offset: usize,
}

/// Check if `line` (after leading whitespace) starts with an uppercase MDX component tag `<Foo...`.
pub(crate) fn starts_with_component_tag(trimmed: &str) -> bool {
    let bytes = trimmed.as_bytes();
    bytes.len() >= 2 && bytes[0] == b'<' && bytes[1].is_ascii_uppercase()
}

/// Parse a declarative `<ComponentName ... />` or `<ComponentName ...>` starting at byte 0 of `input`.
///
/// `start_pos` is the 1-based `(line, column)` of byte 0 in the original document.
pub(crate) fn parse_open_tag(
    input: &str,
    start_pos: SourcePos,
    fm: &Frontmatter,
) -> Option<ParsedOpenTag> {
    let bytes = input.as_bytes();
    if bytes.len() < 2 || bytes[0] != b'<' || !bytes[1].is_ascii_uppercase() {
        return None;
    }

    let mut cur = Cursor::new(input, start_pos);
    cur.advance_byte(); // '<'
    let name_start = cur.idx;
    while let Some(b) = cur.peek_byte() {
        if b.is_ascii_alphanumeric() || b == b'_' || b == b'.' || b == b'-' {
            cur.advance_byte();
        } else {
            break;
        }
    }
    let name = input[name_start..cur.idx].to_string();
    if name.is_empty() {
        return None;
    }

    let mut props = Vec::new();
    loop {
        cur.skip_ws();
        let b = cur.peek_byte()?;
        if b == b'>' {
            cur.advance_byte();
            return Some(ParsedOpenTag {
                name,
                props,
                self_closing: false,
                pos: start_pos,
                end_offset: cur.idx,
            });
        }
        if b == b'/' && cur.peek_byte_at(1) == Some(b'>') {
            cur.advance_byte();
            cur.advance_byte();
            return Some(ParsedOpenTag {
                name,
                props,
                self_closing: true,
                pos: start_pos,
                end_offset: cur.idx,
            });
        }

        let prop_pos = cur.pos;
        let prop_name = cur.read_ident()?;
        cur.skip_ws();
        let value = if cur.peek_byte() == Some(b'=') {
            cur.advance_byte();
            cur.skip_ws();
            parse_prop_value(&mut cur, fm)?
        } else {
            // Bare boolean flag: `<Card featured />`
            PropValue::Bool(true)
        };
        props.push(ComponentProp {
            name: prop_name,
            value,
            pos: prop_pos,
        });
    }
}

fn parse_prop_value(cur: &mut Cursor<'_>, fm: &Frontmatter) -> Option<PropValue> {
    let b = cur.peek_byte()?;
    match b {
        b'"' | b'\'' => Some(PropValue::String(cur.read_quoted_string()?)),
        b'{' => {
            cur.advance_byte();
            let start = cur.idx;
            let mut depth = 1usize;
            let mut in_single = false;
            let mut in_double = false;
            let mut escape = false;
            while let Some(ch) = cur.peek_char() {
                if escape {
                    escape = false;
                    cur.advance_char();
                    continue;
                }
                if ch == '\\' && (in_single || in_double) {
                    escape = true;
                    cur.advance_char();
                    continue;
                }
                match ch {
                    '\'' if !in_double => in_single = !in_single,
                    '"' if !in_single => in_double = !in_double,
                    '{' if !in_single && !in_double => depth += 1,
                    '}' if !in_single && !in_double => {
                        depth -= 1;
                        if depth == 0 {
                            let expr = cur.slice_from(start).trim();
                            cur.advance_char();
                            return Some(eval_braced_expr(expr, fm));
                        }
                    }
                    _ => {}
                }
                cur.advance_char();
            }
            None
        }
        _ => {
            // Unquoted bare token fallback (e.g. `count=42`).
            let start = cur.idx;
            while let Some(ch) = cur.peek_char() {
                if ch.is_whitespace() || ch == '>' || ch == '/' {
                    break;
                }
                cur.advance_char();
            }
            let raw = cur.slice_from(start);
            Some(eval_braced_expr(raw, fm))
        }
    }
}

fn eval_braced_expr(expr: &str, fm: &Frontmatter) -> PropValue {
    let trimmed = expr.trim();
    if trimmed == "true" {
        return PropValue::Bool(true);
    }
    if trimmed == "false" {
        return PropValue::Bool(false);
    }
    if let Some(key) = trimmed.strip_prefix("frontmatter.") {
        if let Some(val) = fm.resolve_prop(key.trim()) {
            return val;
        }
        return PropValue::String(String::new());
    }
    if (trimmed.starts_with('"') && trimmed.ends_with('"') && trimmed.len() >= 2)
        || (trimmed.starts_with('\'') && trimmed.ends_with('\'') && trimmed.len() >= 2)
    {
        return PropValue::String(unescape_quoted(&trimmed[1..trimmed.len() - 1]));
    }
    let num_clean = trimmed.replace('_', "");
    if let Ok(i) = num_clean.parse::<i64>() {
        return PropValue::Int(i);
    }
    if let Ok(f) = num_clean.parse::<f64>()
        && f.is_finite()
    {
        return PropValue::Float(f);
    }
    PropValue::String(trimmed.to_string())
}

fn unescape_quoted(inner: &str) -> String {
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('r') => out.push('\r'),
                Some('t') => out.push('\t'),
                Some('"') => out.push('"'),
                Some('\'') => out.push('\''),
                Some('\\') => out.push('\\'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(ch);
        }
    }
    out
}

/// Parse a `{key="val" key2=val2 flag}` attribute block on a `:::` directive header.
pub(crate) fn parse_directive_attrs(raw: &str) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    let trimmed = raw.trim();
    let inner = trimmed
        .strip_prefix('{')
        .and_then(|s| s.strip_suffix('}'))
        .unwrap_or(trimmed);
    let mut cur = Cursor::new(inner, SourcePos::default());

    loop {
        cur.skip_ws();
        if cur.is_eof() {
            break;
        }
        let Some(key) = cur.read_ident() else {
            cur.advance_char();
            continue;
        };
        cur.skip_ws();
        if cur.peek_byte() == Some(b'=') {
            cur.advance_byte();
            cur.skip_ws();
            let val = match cur.peek_byte() {
                Some(b'"' | b'\'') => cur.read_quoted_string().unwrap_or_default(),
                _ => {
                    let start = cur.idx;
                    while let Some(ch) = cur.peek_char() {
                        if ch.is_whitespace() || ch == '}' || ch == ',' {
                            break;
                        }
                        cur.advance_char();
                    }
                    cur.slice_from(start).to_string()
                }
            };
            map.insert(key, val);
        } else {
            map.insert(key, "true".to_string());
        }
    }
    map
}

/// Parse an inline self-closing `<Component ... />` at byte 0 of `input`.
pub(crate) fn parse_inline_component(
    input: &str,
    pos: SourcePos,
    fm: &Frontmatter,
) -> Option<(ComponentNode, usize)> {
    let tag = parse_open_tag(input, pos, fm)?;
    if !tag.self_closing {
        return None;
    }
    Some((
        ComponentNode {
            name: tag.name,
            props: tag.props,
            children: Vec::new(),
            self_closing: true,
            pos: tag.pos,
        },
        tag.end_offset,
    ))
}

struct Cursor<'a> {
    src: &'a str,
    idx: usize,
    pos: SourcePos,
}

impl<'a> Cursor<'a> {
    fn new(src: &'a str, pos: SourcePos) -> Self {
        Self { src, idx: 0, pos }
    }

    fn is_eof(&self) -> bool {
        self.idx >= self.src.len()
    }

    fn peek_byte(&self) -> Option<u8> {
        self.src.as_bytes().get(self.idx).copied()
    }

    fn peek_byte_at(&self, delta: usize) -> Option<u8> {
        self.src.as_bytes().get(self.idx + delta).copied()
    }

    fn peek_char(&self) -> Option<char> {
        self.src[self.idx..].chars().next()
    }

    fn advance_byte(&mut self) {
        if let Some(&b) = self.src.as_bytes().get(self.idx) {
            self.idx += 1;
            if b == b'\n' {
                self.pos.line += 1;
                self.pos.column = 1;
            } else {
                self.pos.column += 1;
            }
        }
    }

    fn advance_char(&mut self) {
        if let Some(ch) = self.peek_char() {
            self.idx += ch.len_utf8();
            if ch == '\n' {
                self.pos.line += 1;
                self.pos.column = 1;
            } else {
                self.pos.column += 1;
            }
        }
    }

    fn skip_ws(&mut self) {
        while let Some(ch) = self.peek_char() {
            if ch.is_whitespace() {
                self.advance_char();
            } else {
                break;
            }
        }
    }

    fn slice_from(&self, start: usize) -> &'a str {
        &self.src[start..self.idx]
    }

    fn read_ident(&mut self) -> Option<String> {
        let start = self.idx;
        let first = self.peek_byte()?;
        if !(first.is_ascii_alphabetic() || first == b'_') {
            return None;
        }
        while let Some(b) = self.peek_byte() {
            if b.is_ascii_alphanumeric() || b == b'_' || b == b'-' || b == b':' {
                self.advance_byte();
            } else {
                break;
            }
        }
        Some(self.src[start..self.idx].to_string())
    }

    fn read_quoted_string(&mut self) -> Option<String> {
        let quote = self.peek_byte()?;
        if quote != b'"' && quote != b'\'' {
            return None;
        }
        self.advance_byte();
        let start = self.idx;
        let mut escape = false;
        while let Some(ch) = self.peek_char() {
            if escape {
                escape = false;
                self.advance_char();
                continue;
            }
            if ch == '\\' {
                escape = true;
                self.advance_char();
                continue;
            }
            if ch as u32 == quote as u32 {
                let raw = self.slice_from(start);
                self.advance_char();
                return Some(unescape_quoted(raw));
            }
            self.advance_char();
        }
        None
    }
}
