//! Zero-serialization MDN `Sec-CH-*` Client Hints parser (RFC 8941 Structured Field syntax)
//! for Server-Side Rendering (SSR) viewport, color-scheme, reduced-motion, DPR, and mobile
//! adaptation before any client JavaScript/WASM executes.
//!
//! Reference:
//! - <https://developer.mozilla.org/en-US/docs/Web/HTTP/Guides/Client_hints>
//! - <https://www.rfc-editor.org/rfc/rfc8941.html>

use nve_abi::num::parse_f32;

use crate::media::MediaEnv;

/// `Accept-CH` response header opting browsers into sending user-preference, viewport, DPR,
/// and device-class Client Hints on subsequent requests.
pub const SSR_ACCEPT_CH_HEADER: &str = "Sec-CH-Prefers-Color-Scheme, Sec-CH-Prefers-Reduced-Motion, Sec-CH-Prefers-Reduced-Transparency, Sec-CH-Viewport-Width, Sec-CH-Viewport-Height, Sec-CH-DPR, Sec-CH-UA-Mobile, Sec-CH-UA-Platform";

/// `Critical-CH` response header requesting an immediate transparent retry if color-scheme or
/// viewport-width hints were absent from the first request.
pub const SSR_CRITICAL_CH_HEADER: &str = "Sec-CH-Prefers-Color-Scheme, Sec-CH-Viewport-Width";

/// `Vary` response header ensuring intermediate caches key SSR HTML variants by the hints that
/// affect initial server rendering.
pub const SSR_VARY_HEADER: &str = "Sec-CH-Prefers-Color-Scheme, Sec-CH-Prefers-Reduced-Motion, Sec-CH-Viewport-Width, Sec-CH-UA-Mobile";

const MOBILE_FALLBACK_WIDTH_PT: f32 = 390.0;
const MOBILE_FALLBACK_HEIGHT_PT: f32 = 844.0;
const MAX_PLATFORM_LEN: usize = 64;
const MAX_VIEWPORT_PT: f32 = 65_535.0;
const MAX_DPR: f32 = 16.0;

/// Parsed MDN `Sec-CH-*` Client Hints snapshot driving initial SSR [`MediaEnv`] evaluation.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ClientHintsSnapshot {
    pub media: MediaEnv,
    pub dpr: Option<f32>,
    pub reduced_transparency: Option<bool>,
    pub mobile: Option<bool>,
    pub platform: Option<String>,
    pub save_data: bool,
}

/// Parses an RFC 8941 Structured Field string or bare token value, stripping optional
/// surrounding `"` and unescaping `\"` / `\\` pairs without ever panicking on malformed input.
fn parse_sf_string(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(rest) = trimmed.strip_prefix('"') {
        let inner = rest.strip_suffix('"')?;
        let mut out = String::with_capacity(inner.len().min(MAX_PLATFORM_LEN));
        let mut chars = inner.chars();
        while let Some(ch) = chars.next() {
            if ch == '\\' {
                match chars.next() {
                    Some(escaped @ ('"' | '\\')) => out.push(escaped),
                    _ => return None,
                }
            } else if ch.is_control() {
                return None;
            } else {
                out.push(ch);
            }
            if out.len() > MAX_PLATFORM_LEN {
                return None;
            }
        }
        let cleaned = out.trim();
        if cleaned.is_empty() {
            None
        } else {
            Some(cleaned.to_owned())
        }
    } else {
        if trimmed.len() > MAX_PLATFORM_LEN || trimmed.chars().any(char::is_control) {
            return None;
        }
        Some(trimmed.to_owned())
    }
}

/// Parses an RFC 8941 Structured Field boolean (`?1` or `?0`), tolerating surrounding whitespace
/// or optional quotes.
fn parse_sf_bool(raw: &str) -> Option<bool> {
    let mut s = raw.trim();
    if let Some(inner) = s.strip_prefix('"').and_then(|r| r.strip_suffix('"')) {
        s = inner.trim();
    }
    match s {
        "?1" => Some(true),
        "?0" => Some(false),
        _ => None,
    }
}

/// Parses a positive finite Structured Field numeric hint (`Sec-CH-Viewport-Width`,
/// `Sec-CH-Viewport-Height`, `Sec-CH-DPR`), rejecting non-finite, negative, zero, or out-of-range
/// values.
fn parse_positive_hint_f32(raw: &str, max_value: f32) -> Option<f32> {
    let mut s = raw.trim();
    if let Some(inner) = s.strip_prefix('"').and_then(|r| r.strip_suffix('"')) {
        s = inner.trim();
    }
    let val = parse_f32(s)?;
    if val.is_finite() && val > 0.0 && val <= max_value {
        Some(val)
    } else {
        None
    }
}

impl ClientHintsSnapshot {
    /// Parses MDN `Sec-CH-*` Client Hints from an arbitrary HTTP header iterator.
    ///
    /// Header names are matched ASCII case-insensitively. Malformed, hostile, or out-of-range
    /// values are ignored gracefully and never panic.
    ///
    /// When `Sec-CH-UA-Mobile: ?1` is present and `Sec-CH-Viewport-Width` is absent (or invalid),
    /// [`MediaEnv::width_pt`] falls back to `390.0`, [`MediaEnv::height_pt`] falls back to `844.0`
    /// (unless `Sec-CH-Viewport-Height` was explicitly supplied), and [`MediaEnv::coarse_pointer`]
    /// is set to `true`.
    pub fn from_headers<'a>(headers: impl IntoIterator<Item = (&'a str, &'a str)>) -> Self {
        let mut snap = Self::default();
        let mut has_viewport_width = false;
        let mut has_viewport_height = false;

        for (raw_name, raw_value) in headers {
            let name = raw_name.trim();
            if name.eq_ignore_ascii_case("sec-ch-prefers-color-scheme") {
                if let Some(val) = parse_sf_string(raw_value) {
                    if val.eq_ignore_ascii_case("dark") {
                        snap.media.dark_mode = true;
                    } else if val.eq_ignore_ascii_case("light") {
                        snap.media.dark_mode = false;
                    }
                }
            } else if name.eq_ignore_ascii_case("sec-ch-prefers-reduced-motion") {
                if let Some(val) = parse_sf_string(raw_value) {
                    if val.eq_ignore_ascii_case("reduce") {
                        snap.media.reduced_motion = true;
                    } else if val.eq_ignore_ascii_case("no-preference") {
                        snap.media.reduced_motion = false;
                    }
                }
            } else if name.eq_ignore_ascii_case("sec-ch-prefers-reduced-transparency") {
                if let Some(val) = parse_sf_string(raw_value) {
                    if val.eq_ignore_ascii_case("reduce") {
                        snap.reduced_transparency = Some(true);
                    } else if val.eq_ignore_ascii_case("no-preference") {
                        snap.reduced_transparency = Some(false);
                    }
                }
            } else if name.eq_ignore_ascii_case("sec-ch-viewport-width") {
                if let Some(w) = parse_positive_hint_f32(raw_value, MAX_VIEWPORT_PT) {
                    snap.media.width_pt = w;
                    has_viewport_width = true;
                }
            } else if name.eq_ignore_ascii_case("sec-ch-viewport-height") {
                if let Some(h) = parse_positive_hint_f32(raw_value, MAX_VIEWPORT_PT) {
                    snap.media.height_pt = h;
                    has_viewport_height = true;
                }
            } else if name.eq_ignore_ascii_case("sec-ch-dpr") {
                if let Some(dpr) = parse_positive_hint_f32(raw_value, MAX_DPR) {
                    snap.dpr = Some(dpr);
                }
            } else if name.eq_ignore_ascii_case("sec-ch-ua-mobile") {
                if let Some(is_mobile) = parse_sf_bool(raw_value) {
                    snap.mobile = Some(is_mobile);
                    snap.media.coarse_pointer = is_mobile;
                }
            } else if name.eq_ignore_ascii_case("sec-ch-ua-platform") {
                if let Some(platform) = parse_sf_string(raw_value) {
                    snap.platform = Some(platform);
                }
            } else if name.eq_ignore_ascii_case("save-data")
                && let Some(val) = parse_sf_string(raw_value)
            {
                if val.eq_ignore_ascii_case("on") {
                    snap.save_data = true;
                } else if val.eq_ignore_ascii_case("off") {
                    snap.save_data = false;
                }
            }
        }

        if snap.mobile == Some(true) {
            snap.media.coarse_pointer = true;
            if !has_viewport_width {
                snap.media.width_pt = MOBILE_FALLBACK_WIDTH_PT;
                if !has_viewport_height {
                    snap.media.height_pt = MOBILE_FALLBACK_HEIGHT_PT;
                }
            }
        }

        snap
    }
}

impl MediaEnv {
    /// Builds a [`MediaEnv`] directly from HTTP `Sec-CH-*` Client Hints headers.
    pub fn from_client_hints<'a>(headers: impl IntoIterator<Item = (&'a str, &'a str)>) -> Self {
        ClientHintsSnapshot::from_headers(headers).media
    }
}
