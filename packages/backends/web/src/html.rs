//! HTML escaping and serialization primitives shared by `render_html`, SSR, and `WebTableSystem`.

#[cfg(feature = "ssr")]
use core::fmt::Write as _;

/// Appends `val` escaped for a quoted attribute value (`&`, `"`, `'`, `<`, `>`, and NUL as
/// U+FFFD), copying unescaped runs as whole slices.
pub fn push_attr_escaped(val: &str, out: &mut String) {
    let bytes = val.as_bytes();
    let mut last = 0usize;
    for (idx, &b) in bytes.iter().enumerate() {
        let esc = match b {
            b'&' => "&amp;",
            b'"' => "&quot;",
            b'\'' => "&#39;",
            b'<' => "&lt;",
            b'>' => "&gt;",
            b'\0' => "\u{FFFD}",
            _ => continue,
        };
        out.push_str(&val[last..idx]);
        out.push_str(esc);
        last = idx + 1;
    }
    out.push_str(&val[last..]);
}

/// Appends `val` escaped for element text content.
pub fn push_text_escaped(val: &str, out: &mut String) {
    for ch in val.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            c => out.push(c),
        }
    }
}

/// Appends ` name="escaped value"`.
pub fn write_attr(name: &str, val: &str, out: &mut String) {
    out.push(' ');
    out.push_str(name);
    out.push_str("=\"");
    push_attr_escaped(val, out);
    out.push('"');
}

/// Appends ` nonce="..."` (attribute-escaped) when an opt-in non-empty CSP style nonce is set.
#[cfg(feature = "ssr")]
pub(crate) fn write_style_nonce_attr(nonce: Option<&str>, out: &mut String) {
    if let Some(n) = nonce.filter(|s| !s.is_empty()) {
        write_attr("nonce", n, out);
    }
}

/// Escapes any case-insensitive `</tag` breakout sequence inside raw-text `<style>` / `<script>`
/// blocks as `<\/tag` (zero-overhead pass-through when `<` is absent).
#[cfg(feature = "ssr")]
fn push_raw_text_escaped(text: &str, needle_lower: &[u8], out: &mut String) {
    if !text.contains('<') {
        out.push_str(text);
        return;
    }
    let bytes = text.as_bytes();
    let nlen = needle_lower.len();
    let mut last = 0usize;
    let mut i = 0usize;
    while i + nlen <= bytes.len() {
        if bytes[i] == b'<'
            && bytes[i + 1] == b'/'
            && bytes[i..i + nlen].eq_ignore_ascii_case(needle_lower)
        {
            out.push_str(&text[last..i]);
            out.push_str("<\\/");
            out.push_str(&text[i + 2..i + nlen]);
            i += nlen;
            last = i;
        } else {
            i += 1;
        }
    }
    out.push_str(&text[last..]);
}

/// Appends CSS text into a `<style>` element, escaping any case-insensitive `</style` sequence
/// so untrusted/edge CSS content can never break out of the `<style>` element.
#[cfg(feature = "ssr")]
#[inline]
pub(crate) fn push_style_css_escaped(css: &str, out: &mut String) {
    push_raw_text_escaped(css, b"</style", out);
}

/// Appends inert data into a `<script type="...">` element, rewriting the `<` of every
/// case-insensitive `</script` and every `<!--` as `\u003c` so payloads can neither close the
/// element nor enter the HTML "script data escaped" state, while string contents stay valid JSON
/// and JavaScript (zero-copy when `<` is absent).
#[cfg(feature = "ssr")]
pub fn push_script_data_escaped(data: &str, out: &mut String) {
    if !data.contains('<') {
        out.push_str(data);
        return;
    }
    const CLOSE: &[u8] = b"</script";
    const COMMENT: &[u8] = b"<!--";
    let bytes = data.as_bytes();
    let mut last = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }
        let rest = &bytes[i..];
        let needle_len = if rest.len() >= CLOSE.len() && rest[..CLOSE.len()].eq_ignore_ascii_case(CLOSE)
        {
            CLOSE.len()
        } else if rest.starts_with(COMMENT) {
            COMMENT.len()
        } else {
            i += 1;
            continue;
        };
        out.push_str(&data[last..i]);
        out.push_str("\\u003c");
        last = i + 1;
        i += needle_len;
    }
    out.push_str(&data[last..]);
}

/// Appends one `<link rel="stylesheet" href="...">` line per non-blank `href`.
#[cfg(feature = "ssr")]
pub fn push_stylesheet_links(links: &[String], out: &mut String) {
    for href in links.iter().map(|h| h.trim()).filter(|h| !h.is_empty()) {
        out.push_str("<link rel=\"stylesheet\" href=\"");
        push_attr_escaped(href, out);
        out.push_str("\">\n");
    }
}

/// Appends `<link rel="modulepreload" href="...">` when `src` is non-blank.
#[cfg(feature = "ssr")]
pub fn push_modulepreload_link(src: Option<&str>, out: &mut String) {
    if let Some(src) = src.map(str::trim).filter(|s| !s.is_empty()) {
        out.push_str("<link rel=\"modulepreload\" href=\"");
        push_attr_escaped(src, out);
        out.push_str("\">\n");
    }
}

/// Appends `<script type="module" async src="..."></script>` when `src` is non-blank.
#[cfg(feature = "ssr")]
pub fn push_async_boot_script(src: Option<&str>, out: &mut String) {
    if let Some(src) = src.map(str::trim).filter(|s| !s.is_empty()) {
        out.push_str("<script type=\"module\" async src=\"");
        push_attr_escaped(src, out);
        out.push_str("\"></script>\n");
    }
}

/// Appends an inert Option B query payload with its parser-completion ack:
/// `<script type="application/x-nve-query" data-nve-seq="{seq}"[ data-nve-slot-for="{slot}"]>{b64}</script><template data-nve-q-ack="{seq}"></template>\n`.
///
/// `slot_for` binds a slot chunk's payload to its `<div slot>` markup: clients ingest it only
/// once that slot's `data-nve-slot-ack` arrived (or the promotion adopted the slot), so data and
/// markup always become visible to hydration together.
#[cfg(feature = "ssr")]
pub fn push_query_payload_tag(seq: u64, slot_for: Option<&str>, base64_nveq: &str, out: &mut String) {
    let _ = write!(
        out,
        "<script type=\"application/x-nve-query\" data-nve-seq=\"{seq}\""
    );
    if let Some(slot) = slot_for {
        out.push_str(" data-nve-slot-for=\"");
        push_attr_escaped(slot, out);
        out.push('"');
    }
    out.push('>');
    push_script_data_escaped(base64_nveq, out);
    let _ = writeln!(out, "</script><template data-nve-q-ack=\"{seq}\"></template>");
}

/// Parents whose direct children the HTML5 parser foster-parents out (a streamed DSD `<slot>`
/// placed directly inside them would be relocated, breaking out-of-order slot delivery).
pub const FOSTER_PARENT_TRAP_TAGS: &[&str] = &["table", "tbody", "thead", "tfoot", "tr"];

/// HTML void elements NVE can emit (no content, no end tag).
#[inline]
pub fn is_void_html_tag(tag: &str) -> bool {
    matches!(tag, "img" | "input" | "br" | "hr" | "meta" | "link")
}

/// Appends the end tag `</tag>`.
pub fn close_html_element(tag: &str, out: &mut String) {
    out.push_str("</");
    out.push_str(tag);
    out.push('>');
}
