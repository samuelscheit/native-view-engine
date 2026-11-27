//! Declaration parsing for the host-runtime drift guard: comments are stripped, `(name, value)`
//! declarations are read with identifier / digit boundaries, and a mirror must equal its Rust
//! source exactly — no missing, mismatched, or stale extra entry.

use std::collections::BTreeMap;

/// How one language spells a `(name, value)` entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Syntax {
    /// `Name = 1` (C / C++ / Swift / Java fields), names optionally carrying a C prefix (`NVE_FIELD_`).
    Assign,
    /// `NAME(1)` (Java / Dart enhanced enums).
    Call,
    /// `Name: 1` (TS `as const` objects).
    Colon,
    /// Values are declaration order (`enum E { a, b }`): the list must match exactly, in order.
    Ordered,
}

/// Case-, whitespace- and `_`-insensitive key: `InternalError`, `INTERNAL_ERROR`, `internalError`.
pub fn normalize(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_whitespace() && *c != '_')
        .flat_map(char::to_lowercase)
        .collect()
}

/// Removes `//` line comments and `/* */` block comments (doc comments included).
pub fn strip_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find('/') {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        if tail.starts_with("//") {
            rest = tail.find('\n').map_or("", |n| &tail[n..]);
        } else if tail.starts_with("/*") {
            rest = tail.find("*/").map_or("", |n| &tail[n + 2..]);
        } else {
            out.push('/');
            rest = &tail[1..];
        }
    }
    out.push_str(rest);
    out
}

/// The declaration starting at `marker` through the `}` that closes its first `{` (nested braces
/// balanced), e.g. a whole enum including its methods.
pub fn block<'a>(text: &'a str, marker: &str) -> Option<&'a str> {
    let start = text.find(marker)?;
    let open = start + text[start..].find('{')?;
    let mut depth = 0usize;
    for (i, c) in text[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&text[start..=open + i]);
                }
            }
            _ => {}
        }
    }
    None
}

fn is_ident(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// `(identifier, rest)` when `s` starts with an identifier.
fn ident(s: &str) -> Option<(&str, &str)> {
    let first = s.chars().next()?;
    if !(first.is_ascii_alphabetic() || first == '_') {
        return None;
    }
    let end = s.find(|c: char| !is_ident(c)).unwrap_or(s.len());
    Some((&s[..end], &s[end..]))
}

/// `(value, rest)` for a decimal integer (optional `-`, optional C `u` suffix) that is not
/// followed by another identifier character: `32` never matches inside `320` or `32x`.
fn integer(s: &str) -> Option<(i64, &str)> {
    let s = s.trim_start();
    let (neg, digits) = s.strip_prefix('-').map_or((false, s), |d| (true, d));
    let end = digits
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(digits.len());
    if end == 0 {
        return None;
    }
    let value: i64 = digits[..end].parse().ok()?;
    let rest = digits[end..].strip_prefix('u').unwrap_or(&digits[end..]);
    if rest.starts_with(is_ident) {
        return None;
    }
    Some((if neg { -value } else { value }, rest))
}

/// Every `(raw name, value)` declared in `body` with `syntax` (`Ordered` yields list positions).
pub fn declarations(body: &str, syntax: Syntax) -> Vec<(String, i64)> {
    let body = strip_comments(body);
    if syntax == Syntax::Ordered {
        let Some(open) = body.find('{') else {
            return Vec::new();
        };
        let list = &body[open + 1..];
        let list = &list[..list.find([';', '}']).unwrap_or(list.len())];
        return list
            .split(',')
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .zip(0..)
            .map(|(n, i)| (n.to_owned(), i))
            .collect();
    }
    let (open, close) = match syntax {
        Syntax::Assign => ("=", ""),
        Syntax::Call => ("(", ")"),
        _ => (":", ""),
    };
    let mut out = Vec::new();
    let mut i = 0;
    while i < body.len() {
        let at = &body[i..];
        let starts_token = body[..i].chars().next_back().is_none_or(|c| !is_ident(c));
        if let (true, Some((name, rest))) = (starts_token, ident(at)) {
            let parsed = rest.trim_start().strip_prefix(open).and_then(|r| {
                if open == "=" && r.starts_with('=') {
                    return None; // `==` is a comparison, not a declaration
                }
                let (value, r) = integer(r)?;
                r.trim_start().strip_prefix(close).map(|_| value)
            });
            if let Some(value) = parsed {
                out.push((name.to_owned(), value));
            }
            i += name.len();
        } else {
            i += at.chars().next().map_or(1, char::len_utf8);
        }
    }
    out
}

/// Differences between the mirror `body` and the Rust `expected` set (empty when identical).
///
/// `prefix` is stripped from declared names (e.g. `nve_field_`), `renames` map Rust variant names
/// to the language's spelling, and every expected entry must appear exactly once with its value —
/// a missing, mismatched, duplicated, or stale extra declaration is reported.
pub fn check_block(
    expected: &[(String, i64)],
    syntax: Syntax,
    prefix: &str,
    renames: &[(&str, &str)],
    body: &str,
) -> Vec<String> {
    let rename = |n: &str| {
        renames
            .iter()
            .find(|(from, _)| *from == n)
            .map_or_else(|| n.to_owned(), |(_, to)| (*to).to_owned())
    };
    let want: BTreeMap<String, i64> = expected
        .iter()
        .map(|(n, v)| (normalize(&rename(n)), *v))
        .collect();
    let prefix = normalize(prefix);
    let mut found: BTreeMap<String, i64> = BTreeMap::new();
    let mut problems = Vec::new();
    for (name, value) in declarations(body, syntax) {
        let key = normalize(&name);
        let key = key.strip_prefix(&prefix).map_or(key.clone(), str::to_owned);
        if found.insert(key.clone(), value).is_some() {
            problems.push(format!("`{name}` is declared twice"));
        }
    }
    for (name, value) in &want {
        match found.get(name) {
            None => problems.push(format!("missing `{name}` = {value}")),
            Some(v) if v != value => problems.push(format!("`{name}` is {v}, Rust says {value}")),
            Some(_) => {}
        }
    }
    for (name, value) in &found {
        if !want.contains_key(name) {
            problems.push(format!("stale extra `{name}` = {value} (not in Rust)"));
        }
    }
    problems
}

/// Values of every `name = N` (or `#define name N`) declaration of exactly `name`, comments ignored.
pub fn declared_values(text: &str, name: &str) -> Vec<i64> {
    let text = strip_comments(text);
    let mut out = Vec::new();
    for (at, _) in text.match_indices(name) {
        let before = text[..at].chars().next_back();
        let rest = &text[at + name.len()..];
        if before.is_some_and(is_ident) || rest.starts_with(is_ident) {
            continue;
        }
        let define = text[..at].trim_end().ends_with("#define");
        let value = if define {
            integer(rest).map(|(v, _)| v)
        } else {
            rest.trim_start()
                .strip_prefix('=')
                .filter(|r| !r.starts_with('='))
                .and_then(integer)
                .map(|(v, _)| v)
        };
        out.extend(value);
    }
    out
}

/// `true` when `needle` occurs in the normalized, comment-free `text` and is not followed by
/// another hex digit (a digest is never accepted as the prefix of a longer number).
pub fn contains_hex_token(text: &str, needle: &str) -> bool {
    let hay = normalize(&strip_comments(text));
    hay.match_indices(needle)
        .any(|(at, _)| !hay[at + needle.len()..].starts_with(|c: char| c.is_ascii_hexdigit()))
}
