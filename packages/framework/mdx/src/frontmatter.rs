//! Frontmatter parser supporting `---` (YAML/TOML-subset) and `+++` (TOML-subset) blocks.

use crate::ast::Frontmatter;

/// Parse leading `---` (or `+++`) frontmatter from `source`.
///
/// Returns `(frontmatter, body_slice, body_start_line)` where `body_start_line` is 1-based.
/// Never panics on malformed or unclosed frontmatter; an unclosed delimiter falls back to
/// treating the entire input as the Markdown body.
pub fn parse_frontmatter(source: &str) -> (Frontmatter, &str, usize) {
    let stripped = source.strip_prefix('\u{feff}').unwrap_or(source);
    let (delim, after_open, open_lines) = match consume_opening_fence(stripped) {
        Some(v) => v,
        None => return (Frontmatter::default(), stripped, 1),
    };

    let mut offset = 0usize;
    let mut closing_range = None;

    for (lines_in_fm, raw_line) in after_open.split_inclusive('\n').enumerate() {
        let line_no_eol = raw_line.trim_end_matches(['\r', '\n']);
        if line_no_eol.trim() == delim {
            closing_range = Some((offset, offset + raw_line.len(), lines_in_fm + 1));
            break;
        }
        offset += raw_line.len();
    }

    let Some((fm_end, body_start, closed_lines)) = closing_range else {
        // Unclosed frontmatter fence: treat whole document as body.
        return (Frontmatter::default(), stripped, 1);
    };

    let fm_text = &after_open[..fm_end];
    let body = &after_open[body_start..];
    let body_line = open_lines + closed_lines + 1;
    let fm = parse_frontmatter_body(fm_text);
    (fm, body, body_line)
}

fn consume_opening_fence(input: &str) -> Option<(&'static str, &str, usize)> {
    let first_eol = input.find('\n').map_or(input.len(), |idx| idx + 1);
    let first_line = input[..first_eol].trim_end_matches(['\r', '\n']).trim();
    let delim = if first_line == "---" {
        "---"
    } else if first_line == "+++" {
        "+++"
    } else {
        return None;
    };
    Some((delim, &input[first_eol..], 1))
}

fn parse_frontmatter_body(text: &str) -> Frontmatter {
    let mut fm = Frontmatter::default();
    let lines: Vec<&str> = text.lines().collect();
    let mut idx = 0usize;

    while idx < lines.len() {
        let raw = lines[idx];
        let trimmed = raw.trim();
        idx += 1;

        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        let Some((key, val_raw)) = split_key_value(trimmed) else {
            continue;
        };
        let key = unquote_key(key.trim());
        if key.is_empty() {
            continue;
        }

        let val_trimmed = strip_inline_comment(val_raw.trim()).trim();

        // Multi-line YAML block list:
        // tags:
        //   - rust
        //   - mdx
        if val_trimmed.is_empty() {
            let mut items = Vec::new();
            while idx < lines.len() {
                let next_trimmed = lines[idx].trim();
                if let Some(rest) = next_trimmed.strip_prefix("- ") {
                    items.push(unquote_scalar(strip_inline_comment(rest.trim()).trim()));
                    idx += 1;
                } else if next_trimmed.is_empty() || next_trimmed.starts_with('#') {
                    idx += 1;
                } else {
                    break;
                }
            }
            if !items.is_empty() {
                apply_array(&mut fm, &key, items);
            }
            continue;
        }

        // Inline array: ["a", "b"] or [a, b]
        if val_trimmed.starts_with('[') {
            let mut combined = val_trimmed.to_string();
            while !has_closing_bracket(&combined) && idx < lines.len() {
                combined.push(' ');
                combined.push_str(strip_inline_comment(lines[idx].trim()).trim());
                idx += 1;
            }
            let items = parse_inline_array(&combined);
            apply_array(&mut fm, &key, items);
            continue;
        }

        apply_scalar(&mut fm, &key, val_trimmed);
    }

    fm
}

fn split_key_value(line: &str) -> Option<(&str, &str)> {
    let mut in_single = false;
    let mut in_double = false;
    let mut escape = false;
    for (idx, ch) in line.char_indices() {
        if escape {
            escape = false;
            continue;
        }
        if ch == '\\' && in_double {
            escape = true;
            continue;
        }
        match ch {
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            ':' | '=' if !in_single && !in_double => {
                return Some((&line[..idx], &line[idx + 1..]));
            }
            _ => {}
        }
    }
    None
}

fn unquote_key(raw: &str) -> String {
    if (raw.starts_with('"') && raw.ends_with('"') && raw.len() >= 2)
        || (raw.starts_with('\'') && raw.ends_with('\'') && raw.len() >= 2)
    {
        raw[1..raw.len() - 1].to_string()
    } else {
        raw.to_string()
    }
}

/// Strip a `# ...` comment that appears outside quoted strings.
pub(crate) fn strip_inline_comment(input: &str) -> &str {
    let mut in_single = false;
    let mut in_double = false;
    let mut escape = false;
    for (idx, ch) in input.char_indices() {
        if escape {
            escape = false;
            continue;
        }
        if ch == '\\' && in_double {
            escape = true;
            continue;
        }
        match ch {
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            '#' if !in_single
                && !in_double
                && (idx == 0 || input[..idx].ends_with(char::is_whitespace)) =>
            {
                return &input[..idx];
            }
            _ => {}
        }
    }
    input
}

fn has_closing_bracket(s: &str) -> bool {
    let mut in_single = false;
    let mut in_double = false;
    let mut escape = false;
    for ch in s.chars() {
        if escape {
            escape = false;
            continue;
        }
        if ch == '\\' && in_double {
            escape = true;
            continue;
        }
        match ch {
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            ']' if !in_single && !in_double => return true,
            _ => {}
        }
    }
    false
}

/// Parse an inline `[a, "b", 'c']` array into unquoted strings.
pub(crate) fn parse_inline_array(raw: &str) -> Vec<String> {
    let inner = raw.trim();
    let inner = inner.strip_prefix('[').unwrap_or(inner);
    let inner = inner.strip_suffix(']').unwrap_or(inner);

    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_single = false;
    let mut in_double = false;
    let mut escape = false;

    for ch in inner.chars() {
        if escape {
            cur.push(ch);
            escape = false;
            continue;
        }
        if ch == '\\' && in_double {
            cur.push(ch);
            escape = true;
            continue;
        }
        match ch {
            '\'' if !in_double => {
                in_single = !in_single;
                cur.push(ch);
            }
            '"' if !in_single => {
                in_double = !in_double;
                cur.push(ch);
            }
            ',' if !in_single && !in_double => {
                let item = unquote_scalar(cur.trim());
                if !item.is_empty() {
                    out.push(item);
                }
                cur.clear();
            }
            _ => cur.push(ch),
        }
    }
    let last = unquote_scalar(cur.trim());
    if !last.is_empty() {
        out.push(last);
    }
    out
}

/// Unquote a single- or double-quoted scalar, decoding common escape sequences.
pub(crate) fn unquote_scalar(raw: &str) -> String {
    let s = raw.trim();
    if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
        let mut out = String::with_capacity(s.len() - 2);
        let mut chars = s[1..s.len() - 1].chars();
        while let Some(ch) = chars.next() {
            if ch == '\\' {
                match chars.next() {
                    Some('n') => out.push('\n'),
                    Some('r') => out.push('\r'),
                    Some('t') => out.push('\t'),
                    Some('"') => out.push('"'),
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
        return out;
    }
    if s.len() >= 2 && s.starts_with('\'') && s.ends_with('\'') {
        return s[1..s.len() - 1].replace("''", "'");
    }
    s.to_string()
}

fn parse_bool(raw: &str) -> Option<bool> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "true" | "yes" | "on" => Some(true),
        "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

fn apply_array(fm: &mut Frontmatter, key: &str, items: Vec<String>) {
    if key == "tags" {
        fm.tags = items;
    } else {
        fm.arrays.insert(key.to_string(), items);
    }
}

fn apply_scalar(fm: &mut Frontmatter, key: &str, raw: &str) {
    let unquoted = unquote_scalar(raw);
    match key {
        "title" => fm.title = Some(unquoted),
        "description" => fm.description = Some(unquoted),
        "order" => {
            if let Ok(n) = unquoted.replace('_', "").parse::<i64>() {
                fm.order = Some(n);
            }
        }
        "badge" => fm.badge = Some(unquoted),
        "icon" => fm.icon = Some(unquoted),
        "toc" => {
            if let Some(b) = parse_bool(&unquoted) {
                fm.toc = b;
            }
        }
        "draft" => {
            if let Some(b) = parse_bool(&unquoted) {
                fm.draft = b;
            }
        }
        "canonical" => fm.canonical = Some(unquoted),
        other => {
            fm.meta.insert(other.to_string(), unquoted);
        }
    }
}
