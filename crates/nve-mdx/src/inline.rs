//! Pure-Rust inline Markdown + GFM + `{frontmatter.*}` interpolation parser.

use crate::ast::{Frontmatter, Inline, SourcePos};
use crate::mdx_tag::parse_inline_component;

const MAX_INLINE_DEPTH: usize = 32;

/// Parse inline Markdown elements starting at `base_pos` (1-based line & column).
pub fn parse_inlines(input: &str, base_pos: SourcePos, fm: &Frontmatter) -> Vec<Inline> {
    parse_inlines_depth(input, base_pos, fm, 0)
}

/// Extract concatenated plain text from a slice of inline nodes.
pub fn inline_plain_text(inlines: &[Inline]) -> String {
    let mut out = String::new();
    append_inline_plain_text(inlines, &mut out);
    out
}

pub(crate) fn append_inline_plain_text(inlines: &[Inline], out: &mut String) {
    for node in inlines {
        match node {
            Inline::Text(s) | Inline::Code(s) => out.push_str(s),
            Inline::Strong(children)
            | Inline::Emphasis(children)
            | Inline::Strikethrough(children)
            | Inline::Link { text: children, .. } => append_inline_plain_text(children, out),
            Inline::Image { alt, .. } => out.push_str(alt),
            Inline::Autolink { url, .. } => out.push_str(url),
            Inline::HardBreak => out.push(' '),
            Inline::InlineComponent(_) => {}
        }
    }
}

fn parse_inlines_depth(
    input: &str,
    base_pos: SourcePos,
    fm: &Frontmatter,
    depth: usize,
) -> Vec<Inline> {
    if depth >= MAX_INLINE_DEPTH {
        return if input.is_empty() {
            Vec::new()
        } else {
            vec![Inline::Text(input.to_string())]
        };
    }

    let mut out: Vec<Inline> = Vec::new();
    let mut text_buf = String::new();
    let bytes = input.as_bytes();
    let len = bytes.len();
    let mut i = 0usize;
    let mut line = base_pos.line;
    let mut col = base_pos.column;

    let flush_text = |buf: &mut String, out: &mut Vec<Inline>| {
        if !buf.is_empty() {
            out.push(Inline::Text(std::mem::take(buf)));
        }
    };

    while i < len {
        let rest = &input[i..];
        let b = bytes[i];

        // Backslash escape for ASCII punctuation or hard break (`\<newline>`).
        if b == b'\\' && i + 1 < len {
            let next = bytes[i + 1];
            if next == b'\n' {
                flush_text(&mut text_buf, &mut out);
                out.push(Inline::HardBreak);
                i += 2;
                line += 1;
                col = 1;
                continue;
            }
            if next.is_ascii_punctuation() {
                text_buf.push(next as char);
                i += 2;
                col += 2;
                continue;
            }
        }

        // Trailing double-space hard line break.
        if b == b' ' && rest.starts_with("  \n") {
            flush_text(&mut text_buf, &mut out);
            out.push(Inline::HardBreak);
            i += 3;
            line += 1;
            col = 1;
            continue;
        }

        // Inline code span: `...` or ``...`` (any N matching backticks).
        if b == b'`'
            && let Some((code, consumed)) = parse_code_span(rest)
        {
            flush_text(&mut text_buf, &mut out);
            out.push(Inline::Code(code));
            advance_pos(rest, consumed, &mut line, &mut col);
            i += consumed;
            continue;
        }

        // `{frontmatter.key}` simple interpolation.
        if b == b'{'
            && let Some((resolved, consumed)) = parse_frontmatter_expr(rest, fm)
        {
            text_buf.push_str(&resolved);
            advance_pos(rest, consumed, &mut line, &mut col);
            i += consumed;
            continue;
        }

        // Image: `![alt](url "title")`
        if b == b'!'
            && rest.starts_with("![")
            && let Some((alt, url, title, consumed)) = parse_link_or_image(&rest[1..])
        {
            flush_text(&mut text_buf, &mut out);
            out.push(Inline::Image {
                alt: alt.to_string(),
                url,
                title,
                pos: SourcePos::new(line, col),
            });
            advance_pos(rest, consumed + 1, &mut line, &mut col);
            i += consumed + 1;
            continue;
        }

        // Link: `[text](url "title")`
        if b == b'['
            && let Some((label, url, title, consumed)) = parse_link_or_image(rest)
        {
            flush_text(&mut text_buf, &mut out);
            let link_pos = SourcePos::new(line, col);
            let label_pos = SourcePos::new(line, col + 1);
            let children = parse_inlines_depth(label, label_pos, fm, depth + 1);
            out.push(Inline::Link {
                text: children,
                url,
                title,
                pos: link_pos,
            });
            advance_pos(rest, consumed, &mut line, &mut col);
            i += consumed;
            continue;
        }

        // Autolink `<https://...>` / `<mailto:...>` or inline self-closing `<Badge />`.
        if b == b'<' {
            if let Some((url, consumed)) = parse_angle_autolink(rest) {
                flush_text(&mut text_buf, &mut out);
                out.push(Inline::Autolink {
                    url,
                    pos: SourcePos::new(line, col),
                });
                advance_pos(rest, consumed, &mut line, &mut col);
                i += consumed;
                continue;
            }
            if let Some((comp, consumed)) =
                parse_inline_component(rest, SourcePos::new(line, col), fm)
            {
                flush_text(&mut text_buf, &mut out);
                out.push(Inline::InlineComponent(comp));
                advance_pos(rest, consumed, &mut line, &mut col);
                i += consumed;
                continue;
            }
        }

        // Bare GFM autolink `https://...` or `http://...`.
        if b == b'h'
            && (rest.starts_with("https://") || rest.starts_with("http://"))
            && (i == 0 || !bytes[i - 1].is_ascii_alphanumeric())
            && let Some((url, consumed)) = parse_bare_autolink(rest)
        {
            flush_text(&mut text_buf, &mut out);
            out.push(Inline::Autolink {
                url,
                pos: SourcePos::new(line, col),
            });
            advance_pos(rest, consumed, &mut line, &mut col);
            i += consumed;
            continue;
        }

        // Strikethrough `~~...~~`.
        if rest.starts_with("~~")
            && let Some((inner, consumed)) = find_delimited(rest, "~~", false)
        {
            flush_text(&mut text_buf, &mut out);
            let child_pos = SourcePos::new(line, col + 2);
            let children = parse_inlines_depth(inner, child_pos, fm, depth + 1);
            out.push(Inline::Strikethrough(children));
            advance_pos(rest, consumed, &mut line, &mut col);
            i += consumed;
            continue;
        }

        // Strong `**...**` or `__...__`.
        if (rest.starts_with("**") || (rest.starts_with("__") && !prev_is_alnum(input, i)))
            && let Some((inner, consumed)) =
                find_delimited(rest, &rest[..2], rest.starts_with("__"))
        {
            flush_text(&mut text_buf, &mut out);
            let child_pos = SourcePos::new(line, col + 2);
            let children = parse_inlines_depth(inner, child_pos, fm, depth + 1);
            out.push(Inline::Strong(children));
            advance_pos(rest, consumed, &mut line, &mut col);
            i += consumed;
            continue;
        }

        // Emphasis `*...*` or `_..._`.
        if ((b == b'*' && !rest.starts_with("**"))
            || (b == b'_' && !rest.starts_with("__") && !prev_is_alnum(input, i)))
            && let Some((inner, consumed)) = find_delimited(rest, &rest[..1], b == b'_')
        {
            flush_text(&mut text_buf, &mut out);
            let child_pos = SourcePos::new(line, col + 1);
            let children = parse_inlines_depth(inner, child_pos, fm, depth + 1);
            out.push(Inline::Emphasis(children));
            advance_pos(rest, consumed, &mut line, &mut col);
            i += consumed;
            continue;
        }

        // Regular UTF-8 character.
        let ch = rest.chars().next().unwrap();
        text_buf.push(ch);
        i += ch.len_utf8();
        if ch == '\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }

    flush_text(&mut text_buf, &mut out);
    out
}

fn prev_is_alnum(s: &str, idx: usize) -> bool {
    s[..idx]
        .chars()
        .next_back()
        .is_some_and(char::is_alphanumeric)
}

fn advance_pos(slice: &str, consumed: usize, line: &mut usize, col: &mut usize) {
    for ch in slice[..consumed].chars() {
        if ch == '\n' {
            *line += 1;
            *col = 1;
        } else {
            *col += 1;
        }
    }
}

fn parse_code_span(rest: &str) -> Option<(String, usize)> {
    let ticks = rest.bytes().take_while(|&b| b == b'`').count();
    if ticks == 0 {
        return None;
    }
    let after = &rest[ticks..];
    let bytes = after.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'`' {
            let run = bytes[i..].iter().take_while(|&&b| b == b'`').count();
            if run == ticks {
                let raw = &after[..i];
                // CommonMark strips one leading and one trailing space if both exist and non-blank.
                let normalized = if raw.len() >= 2
                    && raw.starts_with(' ')
                    && raw.ends_with(' ')
                    && raw.bytes().any(|b| b != b' ')
                {
                    &raw[1..raw.len() - 1]
                } else {
                    raw
                };
                return Some((normalized.replace('\n', " "), ticks + i + ticks));
            }
            i += run;
        } else {
            i += 1;
        }
    }
    None
}

fn parse_frontmatter_expr(rest: &str, fm: &Frontmatter) -> Option<(String, usize)> {
    let close = rest.find('}')?;
    let inner = rest[1..close].trim();
    let key = inner.strip_prefix("frontmatter.")?.trim();
    if key.is_empty() || !key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return None;
    }
    let resolved = fm.resolve_str(key).unwrap_or_default();
    Some((resolved, close + 1))
}

fn parse_link_or_image(rest: &str) -> Option<(&str, String, Option<String>, usize)> {
    if !rest.starts_with('[') {
        return None;
    }
    let bytes = rest.as_bytes();
    let mut depth = 0usize;
    let mut close_bracket = None;
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            i += 2;
            continue;
        }
        if bytes[i] == b'`'
            && let Some((_, span)) = parse_code_span(&rest[i..])
        {
            i += span;
            continue;
        }
        match bytes[i] {
            b'[' => depth += 1,
            b']' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    close_bracket = Some(i);
                    break;
                }
            }
            _ => {}
        }
        i += 1;
    }

    let close_bracket = close_bracket?;
    if bytes.get(close_bracket + 1) != Some(&b'(') {
        return None;
    }

    let dest_start = close_bracket + 2;
    let mut paren_depth = 1usize;
    let mut in_quote = false;
    let mut j = dest_start;
    let mut close_paren = None;
    while j < bytes.len() {
        if bytes[j] == b'\\' && j + 1 < bytes.len() {
            j += 2;
            continue;
        }
        match bytes[j] {
            b'"' => in_quote = !in_quote,
            b'(' if !in_quote => paren_depth += 1,
            b')' if !in_quote => {
                paren_depth -= 1;
                if paren_depth == 0 {
                    close_paren = Some(j);
                    break;
                }
            }
            _ => {}
        }
        j += 1;
    }
    let close_paren = close_paren?;
    let label = &rest[1..close_bracket];
    let dest_raw = rest[dest_start..close_paren].trim();
    let (url, title) = split_url_and_title(dest_raw);
    Some((label, url, title, close_paren + 1))
}

fn split_url_and_title(raw: &str) -> (String, Option<String>) {
    let trimmed = raw.trim();
    if let Some(after_lt) = trimmed.strip_prefix('<')
        && let Some(gt) = after_lt.find('>')
    {
        let url = after_lt[..gt].to_string();
        let rem = after_lt[gt + 1..].trim();
        let title = strip_quotes(rem);
        return (url, title);
    }
    if let Some(ws) = trimmed.find(char::is_whitespace) {
        let url = trimmed[..ws].to_string();
        let rem = trimmed[ws..].trim();
        let title = strip_quotes(rem);
        (url, title)
    } else {
        (trimmed.to_string(), None)
    }
}

fn strip_quotes(s: &str) -> Option<String> {
    if s.len() >= 2
        && ((s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\'')))
    {
        Some(s[1..s.len() - 1].to_string())
    } else {
        None
    }
}

fn parse_angle_autolink(rest: &str) -> Option<(String, usize)> {
    let close = rest.find('>')?;
    let inner = &rest[1..close];
    if inner.contains(char::is_whitespace) || inner.contains('<') {
        return None;
    }
    if inner.starts_with("http://")
        || inner.starts_with("https://")
        || inner.starts_with("mailto:")
    {
        Some((inner.to_string(), close + 1))
    } else {
        None
    }
}

fn parse_bare_autolink(rest: &str) -> Option<(String, usize)> {
    let mut end = 0usize;
    for (idx, ch) in rest.char_indices() {
        if ch.is_whitespace() || matches!(ch, '<' | '>' | '"' | '\'') {
            break;
        }
        end = idx + ch.len_utf8();
    }
    // Trim trailing sentence punctuation (`.,;:!?)`).
    while end > 0 {
        let last = rest[..end].chars().next_back()?;
        if matches!(last, '.' | ',' | ';' | ':' | '!' | '?' | ')') {
            end -= last.len_utf8();
        } else {
            break;
        }
    }
    if end <= "http://".len() {
        return None;
    }
    Some((rest[..end].to_string(), end))
}

fn find_delimited<'a>(
    rest: &'a str,
    delim: &str,
    forbid_intraword: bool,
) -> Option<(&'a str, usize)> {
    let dlen = delim.len();
    let after = &rest[dlen..];
    // Opening delimiter cannot be immediately followed by whitespace.
    if after.starts_with(char::is_whitespace) || after.is_empty() {
        return None;
    }
    let bytes = after.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            i += 2;
            continue;
        }
        if bytes[i] == b'`'
            && let Some((_, span)) = parse_code_span(&after[i..])
        {
            i += span;
            continue;
        }
        if after[i..].starts_with(delim) {
            let inner = &after[..i];
            let valid_close = !inner.is_empty() && !inner.ends_with(char::is_whitespace);
            let after_close = &after[i + dlen..];
            let intraword_ok = !forbid_intraword
                || !after_close
                    .chars()
                    .next()
                    .is_some_and(char::is_alphanumeric);
            if valid_close && intraword_ok {
                return Some((inner, dlen + i + dlen));
            }
        }
        i += after[i..].chars().next().map_or(1, char::len_utf8);
    }
    None
}
