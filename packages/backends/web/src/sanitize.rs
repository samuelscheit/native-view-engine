//! Security sanitizers for web URLs, custom HTML tags, and HTML attribute name/value pairs.

/// Security helper: allows only safe URL schemes (`https:`, `http:`, `mailto:`, `tel:`, `blob:`,
/// raster-only `data:image/{png,jpeg,jpg,webp,gif,avif};`, or safe relative/fragment URLs).
///
/// Strips WHATWG ASCII tab/LF/CR and trims leading/trailing C0 controls & space before checking,
/// and rejects protocol-relative `//` and `/\` URLs as well as `data:image/svg+xml`.
pub fn is_safe_web_url(url: &str) -> bool {
    let trimmed = url.trim_matches(|c: char| (c as u32) <= 0x20 || c as u32 == 0x7F);
    if trimmed.is_empty() {
        return false;
    }
    let cleaned: String = trimmed
        .chars()
        .filter(|&c| !matches!(c, '\t' | '\n' | '\r'))
        .collect();
    if cleaned.is_empty() {
        return false;
    }
    // Reject protocol-relative (`//evil.com`) and backslash-normalized (`/\evil.com`, `\evil.com`).
    if cleaned.starts_with("//") || cleaned.starts_with("/\\") || cleaned.starts_with('\\') {
        return false;
    }
    if cleaned.starts_with('/')
        || cleaned.starts_with("./")
        || cleaned.starts_with("../")
        || cleaned.starts_with('#')
        || cleaned.starts_with('?')
    {
        return true;
    }
    // Plain relative paths (e.g. `img.png`, `docs/intro`, `a/b:c`) contain no `:` before the first
    // path/query/fragment delimiter (`/`, `?`, `#`).
    match cleaned.find([':', '/', '?', '#']) {
        None => return true,
        Some(idx) if cleaned.as_bytes()[idx] != b':' => return true,
        _ => {}
    }
    let lower = cleaned.to_ascii_lowercase();
    lower.starts_with("https://")
        || lower.starts_with("http://")
        || lower.starts_with("mailto:")
        || lower.starts_with("tel:")
        || lower.starts_with("blob:")
        || lower.starts_with("data:image/png;")
        || lower.starts_with("data:image/jpeg;")
        || lower.starts_with("data:image/jpg;")
        || lower.starts_with("data:image/webp;")
        || lower.starts_with("data:image/gif;")
        || lower.starts_with("data:image/avif;")
}

/// Security helper: validates an HTML element tag name (strict untrimmed ASCII alphabetic start,
/// alphanumeric + `-`), denying scriptable, raw-text, style, embedded, frame, foreignObject, and
/// SVG/MathML breakout tags.
pub fn is_safe_custom_tag(tag: &str) -> bool {
    if tag.is_empty() || tag.len() > 64 || tag.trim() != tag {
        return false;
    }
    let lower = tag.to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        "script"
            | "style"
            | "object"
            | "embed"
            | "applet"
            | "base"
            | "meta"
            | "link"
            | "iframe"
            | "frame"
            | "frameset"
            | "foreignobject"
            | "animate"
            | "set"
            | "template"
            | "noscript"
            | "plaintext"
            | "xmp"
            | "textarea"
            | "title"
            | "svg"
            | "math"
    ) {
        return false;
    }
    let mut chars = tag.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    first.is_ascii_alphabetic()
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// Parses an HTML `srcset` candidate list without splitting inside `data:` URI headers.
fn is_safe_srcset(value: &str) -> bool {
    let mut rest = value;
    let mut count = 0usize;
    loop {
        rest = rest.trim_start_matches(|c: char| c.is_ascii_whitespace());
        if rest.is_empty() {
            return count > 0;
        }
        if rest.starts_with(',') {
            return false;
        }
        let ws_end = rest
            .find(|c: char| c.is_ascii_whitespace())
            .unwrap_or(rest.len());
        let token = &rest[..ws_end];
        let is_data_uri = token
            .get(..5)
            .is_some_and(|p| p.eq_ignore_ascii_case("data:"));

        let (url, had_comma, after) = if is_data_uri {
            if let Some(stripped) = token.strip_suffix(',') {
                (stripped, true, &rest[ws_end..])
            } else {
                (token, false, &rest[ws_end..])
            }
        } else if let Some(comma) = token.find(',') {
            (&token[..comma], true, &rest[comma + 1..])
        } else {
            (token, false, &rest[ws_end..])
        };

        if url.is_empty() || !is_safe_web_url(url) {
            return false;
        }
        count += 1;

        if had_comma {
            rest = after;
            if rest.trim_start_matches(|c: char| c.is_ascii_whitespace()).is_empty() {
                return false;
            }
            continue;
        }

        let after_ws = after.trim_start_matches(|c: char| c.is_ascii_whitespace());
        if after_ws.is_empty() {
            return true;
        }
        if let Some(comma) = after_ws.find(',') {
            rest = &after_ws[comma + 1..];
            if rest.trim_start_matches(|c: char| c.is_ascii_whitespace()).is_empty() {
                return false;
            }
        } else {
            // Trailing descriptor on the final candidate (e.g. `2x` or `640w`).
            return true;
        }
    }
}

/// Security helper: validates an HTML attribute name/value pair (strict untrimmed attribute name;
/// blocks `on*` event handlers, `is`, `srcdoc`, `style`, `class`, all engine-internal `data-nve-*`
/// attributes outside the explicit public allowlist, SVG animation attributes, and unsafe URLs on
/// all URL-bearing attributes including multi-URL `ping` and `data:`-aware `srcset`).
pub fn is_safe_web_attr(name: &str, value: &str) -> bool {
    if name.is_empty() || name.len() > 64 || name.trim() != name {
        return false;
    }
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_alphabetic()
        || !chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ':'))
    {
        return false;
    }
    let lower = name.to_ascii_lowercase();
    if lower.starts_with("data-nve-")
        && !matches!(
            lower.as_str(),
            "data-nve-island"
                | "data-nve-hydrate"
                | "data-nve-hard-nav"
                | "data-nve-stream-slot"
                | "data-nve-dirty"
                | "data-nve-draft-restored"
        )
    {
        return false;
    }
    if lower.starts_with("on")
        || matches!(
            lower.as_str(),
            "is" | "srcdoc"
                | "style"
                | "class"
                | "attributename"
                | "to"
                | "from"
                | "values"
                | "by"
        )
    {
        return false;
    }
    if lower == "ping" {
        let mut any = false;
        for tok in value.split_ascii_whitespace() {
            any = true;
            if !is_safe_web_url(tok) {
                return false;
            }
        }
        return any;
    }
    if matches!(
        lower.as_str(),
        "href"
            | "src"
            | "action"
            | "formaction"
            | "xlink:href"
            | "poster"
            | "cite"
            | "data"
            | "background"
    ) {
        return is_safe_web_url(value);
    }
    if lower == "srcset" {
        return is_safe_srcset(value);
    }
    true
}
