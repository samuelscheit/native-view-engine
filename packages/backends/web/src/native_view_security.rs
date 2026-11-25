//! Web-specific HTML5 tag allowlist, URL scheme sanitization, safe attribute validation,
//! and CSS blur material parameters shared by `WebBackend`'s `NativeView` adapters.

use nve_components::native_view::{
    NATIVE_VIEW_KIND_PROP_ALIAS, NATIVE_VIEW_KIND_PROP_KEY, NATIVE_VIEW_PROPS_PROP_ALIAS,
    NATIVE_VIEW_PROPS_PROP_KEY,
};
#[cfg(feature = "nv-blur")]
use nve_components::native_view::BlurMaterial;

#[inline]
pub fn is_native_view_prop(key: &str) -> bool {
    matches!(
        key,
        NATIVE_VIEW_KIND_PROP_KEY
            | NATIVE_VIEW_KIND_PROP_ALIAS
            | NATIVE_VIEW_PROPS_PROP_KEY
            | NATIVE_VIEW_PROPS_PROP_ALIAS
    )
}

#[inline]
pub fn is_native_kind_prop(key: &str) -> bool {
    matches!(key, NATIVE_VIEW_KIND_PROP_KEY | NATIVE_VIEW_KIND_PROP_ALIAS)
}

#[cfg(feature = "nv-blur")]
pub(crate) fn blur_material_css_params(material: BlurMaterial) -> (f32, &'static str) {
    match material {
        BlurMaterial::UltraThin => (8.0, "0.08"),
        BlurMaterial::Thin => (12.0, "0.14"),
        BlurMaterial::Regular => (20.0, "0.22"),
        BlurMaterial::Thick => (32.0, "0.34"),
        BlurMaterial::Chrome => (24.0, "0.28"),
        BlurMaterial::SystemMaterial => (16.0, "0.18"),
    }
}

#[cfg(feature = "nv-web-element")]
pub fn allowlisted_builtin_web_tag(tag: &str) -> Option<&'static str> {
    match tag {
        "video" => Some("video"),
        "audio" => Some("audio"),
        "svg" => Some("svg"),
        "iframe" => Some("iframe"),
        "canvas" => Some("canvas"),
        "input" => Some("input"),
        "select" => Some("select"),
        "textarea" => Some("textarea"),
        "progress" => Some("progress"),
        "meter" => Some("meter"),
        "details" => Some("details"),
        "summary" => Some("summary"),
        _ => None,
    }
}

/// Validates `"web.<tag>"` against the safe HTML5 allowlist or W3C custom-element rules.
#[cfg(feature = "nv-web-element")]
pub fn validate_web_element_tag(tag: &str) -> Option<(String, Option<&'static str>)> {
    if let Some(st) = allowlisted_builtin_web_tag(tag) {
        return Some((st.to_owned(), Some(st)));
    }
    let bytes = tag.as_bytes();
    if tag.len() < 3
        || tag.len() > 64
        || !tag.contains('-')
        || tag.ends_with('-')
        || !bytes[0].is_ascii_lowercase()
        || !bytes
            .iter()
            .all(|&b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'-' | b'.' | b'_'))
        || matches!(
            tag,
            "annotation-xml"
                | "color-profile"
                | "font-face"
                | "font-face-src"
                | "font-face-uri"
                | "font-face-format"
                | "font-face-name"
                | "missing-glyph"
        )
    {
        return None;
    }
    Some((tag.to_owned(), None))
}

/// Rejects `javascript:`, `vbscript:`, and `data:` URI schemes (even with embedded control/whitespace).
#[cfg(any(feature = "nv-webview", feature = "nv-video", feature = "nv-web-element"))]
pub fn is_safe_web_url(url: &str) -> bool {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return false;
    }
    let norm: String = trimmed
        .chars()
        .filter(|c| !c.is_ascii_control() && !c.is_ascii_whitespace())
        .map(|c| c.to_ascii_lowercase())
        .collect();
    if norm.starts_with("javascript:")
        || norm.starts_with("vbscript:")
        || norm.starts_with("data:")
    {
        return false;
    }
    if norm.starts_with("https://") || norm.starts_with("http://") || norm == "about:blank" {
        return true;
    }
    let sep = norm.find(['/', '?', '#']).unwrap_or(norm.len());
    !norm[..sep].contains(':')
}

/// Validates `WebElement` attributes, rejecting `on*` event handlers, `srcdoc`, `data-nve-id`, and unsafe URIs.
#[cfg(feature = "nv-web-element")]
pub fn is_safe_web_attr(name: &str, value: &str) -> bool {
    let bytes = name.as_bytes();
    if name.is_empty()
        || name.len() > 64
        || !bytes[0].is_ascii_alphabetic()
        || !bytes
            .iter()
            .all(|&b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b':'))
    {
        return false;
    }
    let lower = name.to_ascii_lowercase();
    if lower.starts_with("on") || lower == "srcdoc" || lower == "data-nve-id" {
        return false;
    }
    if matches!(
        lower.as_str(),
        "src" | "href" | "xlink:href" | "action" | "formaction" | "poster" | "data"
    ) {
        return is_safe_web_url(value);
    }
    true
}
