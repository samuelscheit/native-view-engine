//! Block-level syntax helpers (code fences, headings, tables, lists, plain-text extraction).

use crate::ast::{Block, DirectiveBlock, TableAlign};
use crate::inline::append_inline_plain_text;

pub(crate) struct ParsedCodeMeta {
    pub lang: Option<String>,
    pub title: Option<String>,
    pub highlight_lines: Vec<usize>,
    pub show_line_numbers: bool,
    pub live: bool,
}

pub(crate) struct ListMarker<'a> {
    pub indent: usize,
    pub content_indent: usize,
    pub ordered: bool,
    pub number: Option<usize>,
    pub checked: Option<bool>,
    pub rest: &'a str,
}

pub(crate) fn parse_code_fence_open(trimmed: &str) -> Option<(u8, usize, &str)> {
    let first = *trimmed.as_bytes().first()?;
    if first != b'`' && first != b'~' {
        return None;
    }
    let count = trimmed.bytes().take_while(|&b| b == first).count();
    if count < 3 {
        return None;
    }
    let info = &trimmed[count..];
    if first == b'`' && info.contains('`') {
        return None;
    }
    Some((first, count, info))
}

pub(crate) fn parse_code_meta(info: &str) -> ParsedCodeMeta {
    let mut meta = ParsedCodeMeta {
        lang: None,
        title: None,
        highlight_lines: Vec::new(),
        show_line_numbers: false,
        live: false,
    };
    let tokens = tokenize_meta(info.trim());
    for (idx, tok) in tokens.iter().enumerate() {
        if let Some((k, v)) = tok.split_once('=') {
            let val = strip_quotes(v);
            match k.trim() {
                "title" | "filename" | "file" => meta.title = Some(val),
                "highlight" | "hl" => meta.highlight_lines = parse_line_ranges(&val),
                _ => {}
            }
        } else if tok.starts_with('{') && tok.ends_with('}') && tok.len() >= 2 {
            let ranges = parse_line_ranges(&tok[1..tok.len() - 1]);
            if !ranges.is_empty() {
                meta.highlight_lines = ranges;
            }
        } else {
            match tok.as_str() {
                "showLineNumbers" | "lineNumbers" | "linenos" => meta.show_line_numbers = true,
                "live" => meta.live = true,
                other if idx == 0 => meta.lang = Some(other.to_string()),
                _ => {}
            }
        }
    }
    meta
}

fn tokenize_meta(input: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_quote: Option<char> = None;
    for ch in input.chars() {
        if let Some(q) = in_quote {
            cur.push(ch);
            if ch == q {
                in_quote = None;
            }
        } else if ch == '"' || ch == '\'' {
            in_quote = Some(ch);
            cur.push(ch);
        } else if ch.is_whitespace() {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
        } else {
            cur.push(ch);
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn strip_quotes(s: &str) -> String {
    let s = s.trim();
    if s.len() >= 2
        && ((s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\'')))
    {
        s[1..s.len() - 1].to_string()
    } else {
        s.to_string()
    }
}

fn parse_line_ranges(spec: &str) -> Vec<usize> {
    let mut lines = Vec::new();
    for part in spec.split(',') {
        let p = part.trim();
        if let Some((a, b)) = p.split_once('-') {
            if let (Ok(start), Ok(end)) = (a.trim().parse::<usize>(), b.trim().parse::<usize>()) {
                let lo = start.min(end).max(1);
                let hi = start.max(end).min(lo.saturating_add(10_000));
                for n in lo..=hi {
                    if !lines.contains(&n) {
                        lines.push(n);
                    }
                }
            }
        } else if let Ok(n) = p.parse::<usize>()
            && n >= 1
            && !lines.contains(&n)
        {
            lines.push(n);
        }
    }
    lines.sort_unstable();
    lines
}

pub(crate) fn parse_atx_heading(trimmed: &str) -> Option<(u8, &str)> {
    let hashes = trimmed.bytes().take_while(|&b| b == b'#').count();
    if !(1..=6).contains(&hashes) {
        return None;
    }
    let rest = &trimmed[hashes..];
    if !rest.is_empty() && !rest.starts_with([' ', '\t']) {
        return None;
    }
    let mut text = rest.trim();
    let stripped = text.trim_end_matches('#');
    if stripped.len() < text.len() && (stripped.is_empty() || stripped.ends_with([' ', '\t'])) {
        text = stripped.trim();
    }
    Some((hashes as u8, text))
}

pub(crate) fn is_thematic_break(trimmed: &str) -> bool {
    let mut marker: Option<u8> = None;
    let mut count = 0usize;
    for b in trimmed.bytes() {
        if b == b' ' || b == b'\t' {
            continue;
        }
        if matches!(b, b'-' | b'*' | b'_') {
            match marker {
                None => {
                    marker = Some(b);
                    count = 1;
                }
                Some(m) if m == b => count += 1,
                _ => return false,
            }
        } else {
            return false;
        }
    }
    count >= 3
}

pub(crate) fn looks_like_table_row(line: &str) -> bool {
    line.contains('|')
}

pub(crate) fn parse_table_separator(line: &str) -> Option<Vec<TableAlign>> {
    let trimmed = line.trim();
    if !trimmed.contains('|') || !trimmed.contains('-') {
        return None;
    }
    let cells = split_table_cells(trimmed);
    if cells.is_empty() {
        return None;
    }
    let mut aligns = Vec::with_capacity(cells.len());
    for cell in cells {
        let c = cell.trim();
        if c.is_empty() {
            return None;
        }
        let left = c.starts_with(':');
        let right = c.ends_with(':');
        let mid = c.strip_prefix(':').unwrap_or(c);
        let mid = mid.strip_suffix(':').unwrap_or(mid);
        if mid.is_empty() || !mid.bytes().all(|b| b == b'-') {
            return None;
        }
        aligns.push(match (left, right) {
            (true, true) => TableAlign::Center,
            (true, false) => TableAlign::Left,
            (false, true) => TableAlign::Right,
            (false, false) => TableAlign::None,
        });
    }
    Some(aligns)
}

pub(crate) fn split_table_cells(line: &str) -> Vec<String> {
    let mut s = line.trim();
    if let Some(stripped) = s.strip_prefix('|') {
        s = stripped;
    }
    if let Some(stripped) = s.strip_suffix('|')
        && !stripped.ends_with('\\')
    {
        s = stripped;
    }
    let mut cells = Vec::new();
    let mut cur = String::new();
    let mut in_code = false;
    let mut chars = s.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' && chars.peek() == Some(&'|') {
            cur.push('|');
            chars.next();
            continue;
        }
        if ch == '`' {
            in_code = !in_code;
            cur.push(ch);
            continue;
        }
        if ch == '|' && !in_code {
            cells.push(std::mem::take(&mut cur));
        } else {
            cur.push(ch);
        }
    }
    cells.push(cur);
    cells
}

pub(crate) fn count_leading_spaces(line: &str) -> usize {
    line.bytes().take_while(|&b| b == b' ').count()
}

pub(crate) fn parse_list_marker(line: &str) -> Option<ListMarker<'_>> {
    let indent = count_leading_spaces(line);
    let after = &line[indent..];
    let bytes = after.as_bytes();
    if bytes.len() < 2 {
        return None;
    }

    if matches!(bytes[0], b'-' | b'*' | b'+') && (bytes[1] == b' ' || bytes[1] == b'\t') {
        let mut rest = &after[2..];
        let mut checked = None;
        if let Some(r) = rest.strip_prefix("[x] ").or_else(|| rest.strip_prefix("[X] ")) {
            checked = Some(true);
            rest = r;
        } else if let Some(r) = rest.strip_prefix("[ ] ") {
            checked = Some(false);
            rest = r;
        } else if rest == "[x]" || rest == "[X]" {
            checked = Some(true);
            rest = "";
        } else if rest == "[ ]" {
            checked = Some(false);
            rest = "";
        }
        return Some(ListMarker {
            indent,
            content_indent: indent + 2,
            ordered: false,
            number: None,
            checked,
            rest,
        });
    }

    let digits = bytes.iter().take_while(|b| b.is_ascii_digit()).count();
    if (1..=9).contains(&digits)
        && bytes.len() > digits + 1
        && matches!(bytes[digits], b'.' | b')')
        && (bytes[digits + 1] == b' ' || bytes[digits + 1] == b'\t')
    {
        let num = after[..digits].parse::<usize>().ok()?;
        return Some(ListMarker {
            indent,
            content_indent: indent + digits + 2,
            ordered: true,
            number: Some(num),
            checked: None,
            rest: &after[digits + 2..],
        });
    }

    None
}

pub(crate) fn collect_blocks_plain_text(blocks: &[Block]) -> String {
    let mut out = String::new();
    append_blocks_plain_text(blocks, &mut out);
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn append_blocks_plain_text(blocks: &[Block], out: &mut String) {
    for block in blocks {
        match block {
            Block::Heading { text, .. } => {
                out.push_str(text);
                out.push(' ');
            }
            Block::Paragraph { content, .. } => {
                append_inline_plain_text(content, out);
                out.push(' ');
            }
            Block::CodeBlock(cb) => {
                out.push_str(&cb.code);
                out.push(' ');
            }
            Block::List { items, .. } => {
                for item in items {
                    append_blocks_plain_text(&item.blocks, out);
                }
            }
            Block::Table { headers, rows, .. } => {
                for h in headers {
                    append_inline_plain_text(h, out);
                    out.push(' ');
                }
                for row in rows {
                    for cell in row {
                        append_inline_plain_text(cell, out);
                        out.push(' ');
                    }
                }
            }
            Block::Blockquote { blocks, .. } => append_blocks_plain_text(blocks, out),
            Block::ThematicBreak { .. } => {}
            Block::Directive(dir) => match dir {
                DirectiveBlock::Callout { title, blocks, .. }
                | DirectiveBlock::Demo { title, blocks, .. } => {
                    if let Some(t) = title {
                        out.push_str(t);
                        out.push(' ');
                    }
                    append_blocks_plain_text(blocks, out);
                }
                DirectiveBlock::Tabs { items, .. } => {
                    for item in items {
                        out.push_str(&item.label);
                        out.push(' ');
                        append_blocks_plain_text(&item.blocks, out);
                    }
                }
                DirectiveBlock::Steps { items, .. } => {
                    for item in items {
                        out.push_str(&item.title);
                        out.push(' ');
                        append_blocks_plain_text(&item.blocks, out);
                    }
                }
                DirectiveBlock::Cards { items, .. } => {
                    for item in items {
                        out.push_str(&item.title);
                        out.push(' ');
                        if let Some(d) = &item.description {
                            out.push_str(d);
                            out.push(' ');
                        }
                        append_blocks_plain_text(&item.blocks, out);
                    }
                }
                DirectiveBlock::Custom { blocks, .. } => append_blocks_plain_text(blocks, out),
            },
            Block::Component(comp) => append_blocks_plain_text(&comp.children, out),
        }
    }
}
