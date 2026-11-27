//! HTML escaping and serialization primitives shared by `render_html`, SSR, and `WebTableSystem`.

/// Appends `val` escaped for a double-quoted attribute value.
pub fn push_attr_escaped(val: &str, out: &mut String) {
    for ch in val.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            c => out.push(c),
        }
    }
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

/// Appends inert data into a `<script type="...">` element, escaping any case-insensitive
/// `</script` sequence so data payloads can never break out of the `<script>` element.
#[cfg(feature = "ssr")]
#[inline]
pub(crate) fn push_script_data_escaped(data: &str, out: &mut String) {
    push_raw_text_escaped(data, b"</script", out);
}

/// Parents whose direct children the HTML5 parser foster-parents out (a streamed DSD `<slot>`
/// placed directly inside them would be relocated, breaking out-of-order slot delivery).
pub const FOSTER_PARENT_TRAP_TAGS: &[&str] = &["table", "tbody", "thead", "tfoot", "tr", "select"];

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
