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
