//! Shared zero-dependency Rust integer and string literal parsers for `nve-abi-macros`.

pub(crate) fn parse_usize_literal(raw: &str) -> Result<usize, String> {
    let c = raw.replace('_', "");
    let t = c
        .trim_end_matches("usize")
        .trim_end_matches("u64")
        .trim_end_matches("u32")
        .trim_end_matches("u16")
        .trim_end_matches("u8");
    let (r, d) = if let Some(h) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        (16, h)
    } else if let Some(b) = t.strip_prefix("0b").or_else(|| t.strip_prefix("0B")) {
        (2, b)
    } else if let Some(o) = t.strip_prefix("0o").or_else(|| t.strip_prefix("0O")) {
        (8, o)
    } else {
        (10, t)
    };
    usize::from_str_radix(d, r).map_err(|_| format!("invalid integer `{raw}`"))
}

pub(crate) fn parse_i128_literal(raw: &str, neg: bool) -> Result<i128, String> {
    let c = raw.replace('_', "");
    let (t, unsigned_suffix) = if let Some(s) = c
        .strip_suffix("u128")
        .or_else(|| c.strip_suffix("usize"))
        .or_else(|| c.strip_suffix("u64"))
        .or_else(|| c.strip_suffix("u32"))
        .or_else(|| c.strip_suffix("u16"))
        .or_else(|| c.strip_suffix("u8"))
    {
        (s, true)
    } else if let Some(s) = c
        .strip_suffix("i128")
        .or_else(|| c.strip_suffix("isize"))
        .or_else(|| c.strip_suffix("i64"))
        .or_else(|| c.strip_suffix("i32"))
        .or_else(|| c.strip_suffix("i16"))
        .or_else(|| c.strip_suffix("i8"))
    {
        (s, false)
    } else {
        (c.as_str(), false)
    };
    if neg && unsigned_suffix {
        return Err(format!(
            "negative integer literal `-{raw}` cannot have unsigned suffix"
        ));
    }
    let (radix, digits) = if let Some(h) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        (16, h)
    } else if let Some(b) = t.strip_prefix("0b").or_else(|| t.strip_prefix("0B")) {
        (2, b)
    } else if let Some(o) = t.strip_prefix("0o").or_else(|| t.strip_prefix("0O")) {
        (8, o)
    } else {
        (10, t)
    };
    let mag = u128::from_str_radix(digits, radix).map_err(|_| format!("invalid integer `{raw}`"))?;
    if !neg {
        i128::try_from(mag).map_err(|_| format!("integer literal `{raw}` overflows i128"))
    } else if mag == (i128::MAX as u128) + 1 {
        Ok(i128::MIN)
    } else {
        i128::try_from(mag)
            .map(|v| -v)
            .map_err(|_| format!("integer literal `-{raw}` underflows i128"))
    }
}

pub(crate) fn parse_string_literal(raw: &str) -> Result<String, String> {
    if let Some(rest) = raw.strip_prefix('r') {
        let n = rest.bytes().take_while(|&b| b == b'#').count();
        let body = rest[n..]
            .strip_prefix('"')
            .ok_or_else(|| format!("malformed raw string `{raw}`"))?;
        return body
            .strip_suffix(&format!("\"{}", "#".repeat(n)))
            .map(str::to_owned)
            .ok_or_else(|| format!("unterminated raw string `{raw}`"));
    }
    let inner = raw
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .ok_or_else(|| format!("expected string, found `{raw}`"))?;
    let (mut out, mut chars) = (String::with_capacity(inner.len()), inner.chars());
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some('\\') => out.push('\\'),
            Some('"') => out.push('"'),
            Some('\'') => out.push('\''),
            Some('0') => out.push('\0'),
            Some('x') => {
                let hex = format!(
                    "{}{}",
                    chars.next().ok_or("bad \\x")?,
                    chars.next().ok_or("bad \\x")?
                );
                out.push(
                    u8::from_str_radix(&hex, 16).map_err(|_| format!("bad \\x{hex}"))? as char,
                );
            }
            Some('u') if chars.next() == Some('{') => {
                let hex: String = chars
                    .by_ref()
                    .take_while(|&ch| ch != '}')
                    .filter(|&ch| ch != '_')
                    .collect();
                out.push(
                    char::from_u32(
                        u32::from_str_radix(&hex, 16).map_err(|_| format!("bad \\u{{{hex}}}"))?,
                    )
                    .ok_or("invalid Unicode")?,
                );
            }
            Some('\n') => {
                while matches!(chars.clone().next(), Some(' ' | '\t' | '\r' | '\n')) {
                    chars.next();
                }
            }
            other => return Err(format!("unsupported escape `\\{other:?}`")),
        }
    }
    Ok(out)
}
