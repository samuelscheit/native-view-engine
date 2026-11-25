//! Nextra-style `_meta.toml` parser preserving declaration order.

use crate::frontmatter::{strip_inline_comment, unquote_scalar};

/// Single parsed entry from a `_meta.toml` file (declaration order preserved).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetaEntry {
    pub key: String,
    pub title: Option<String>,
    pub badge: Option<String>,
    pub icon: Option<String>,
    pub href: Option<String>,
    pub collapsed: bool,
    pub separator: bool,
    pub hidden: bool,
}

impl MetaEntry {
    fn empty(key: String) -> Self {
        Self {
            key,
            title: None,
            badge: None,
            icon: None,
            href: None,
            collapsed: false,
            separator: false,
            hidden: false,
        }
    }
}

/// Parse a Nextra-style `_meta.toml` file into ordered [`MetaEntry`] values.
///
/// Supports:
/// - `slug = "Display Title"`
/// - `slug = { title = "Guides", badge = "New", collapsed = true, icon = "book" }`
/// - `"---" = "Section Title"` or `sep = "---"` or `sep = { type = "separator", title = "Core" }`
/// - `github = { title = "GitHub", href = "https://github.com/..." }`
/// - `[slug]` table sections with `title = ...`, `collapsed = true`, etc.
pub fn parse_meta_toml(input: &str) -> Vec<MetaEntry> {
    let mut entries: Vec<MetaEntry> = Vec::new();
    let mut active_table: Option<usize> = None;

    for raw_line in input.lines() {
        let stripped = strip_inline_comment(raw_line).trim();
        if stripped.is_empty() {
            continue;
        }

        if stripped.starts_with('[') && stripped.ends_with(']') && !stripped.starts_with("[[") {
            let key = unquote_scalar(stripped[1..stripped.len() - 1].trim());
            if !key.is_empty() {
                entries.push(MetaEntry::empty(key));
                active_table = Some(entries.len() - 1);
            }
            continue;
        }

        let Some((lhs, rhs)) = split_eq_outside_quotes(stripped) else {
            continue;
        };
        let key = unquote_scalar(lhs.trim());
        let val = rhs.trim();
        if key.is_empty() {
            continue;
        }

        if let Some(table_idx) = active_table {
            apply_meta_field(&mut entries[table_idx], &key, val);
            continue;
        }

        if key == "---" || key.starts_with("---") {
            let title = unquote_scalar(val);
            let mut entry = MetaEntry::empty(key);
            entry.separator = true;
            if !title.is_empty() && title != "---" {
                entry.title = Some(title);
            }
            entries.push(entry);
            continue;
        }

        if val.starts_with('{') && val.ends_with('}') {
            let mut entry = MetaEntry::empty(key);
            parse_inline_table(&val[1..val.len() - 1], &mut entry);
            entries.push(entry);
            continue;
        }

        let scalar = unquote_scalar(val);
        let mut entry = MetaEntry::empty(key);
        if scalar == "---" {
            entry.separator = true;
        } else {
            entry.title = Some(scalar);
        }
        entries.push(entry);
    }

    entries
}

fn split_eq_outside_quotes(s: &str) -> Option<(&str, &str)> {
    let mut in_single = false;
    let mut in_double = false;
    let mut escape = false;
    for (idx, ch) in s.char_indices() {
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
            '=' if !in_single && !in_double => return Some((&s[..idx], &s[idx + 1..])),
            _ => {}
        }
    }
    None
}

fn parse_inline_table(inner: &str, entry: &mut MetaEntry) {
    let mut cur = String::new();
    let mut in_single = false;
    let mut in_double = false;
    let mut escape = false;
    let mut pairs = Vec::new();
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
            ',' if !in_single && !in_double => pairs.push(std::mem::take(&mut cur)),
            _ => cur.push(ch),
        }
    }
    if !cur.trim().is_empty() {
        pairs.push(cur);
    }
    for pair in pairs {
        if let Some((k, v)) = split_eq_outside_quotes(pair.trim()) {
            apply_meta_field(entry, &unquote_scalar(k.trim()), v.trim());
        }
    }
}

fn apply_meta_field(entry: &mut MetaEntry, key: &str, raw_val: &str) {
    let val = unquote_scalar(raw_val);
    match key {
        "title" => entry.title = Some(val),
        "badge" => entry.badge = Some(val),
        "icon" => entry.icon = Some(val),
        "href" | "url" => entry.href = Some(val),
        "collapsed" | "defaultCollapsed" => entry.collapsed = val.eq_ignore_ascii_case("true"),
        "separator" => entry.separator = val.eq_ignore_ascii_case("true"),
        "type" if val.eq_ignore_ascii_case("separator") => entry.separator = true,
        "hidden" | "display"
            if val.eq_ignore_ascii_case("true") || val.eq_ignore_ascii_case("hidden") =>
        {
            entry.hidden = true;
        }
        _ => {}
    }
}
