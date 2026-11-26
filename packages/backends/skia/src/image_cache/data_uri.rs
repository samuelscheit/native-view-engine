//! Zero-dependency `data:` URI, `file://` URI, percent-encoding, and base64 image decoders.

use std::path::PathBuf;

use super::png::decode_png;
use super::{DecodedImage, ImageDecodeError};

/// Decodes a `data:image/png;base64,...`, `data:image/rgba;width=W;height=H;base64,...`,
/// or `file://...` image URI into a [`DecodedImage`].
pub fn decode_image_uri(src: &str) -> Result<DecodedImage, ImageDecodeError> {
    let trimmed = src.trim();
    if let Some(rest) = strip_prefix_ascii_ci(trimmed, "data:") {
        let Some((meta, payload)) = rest.split_once(',') else {
            return Err(ImageDecodeError::InvalidDataUri);
        };
        let mut parts = meta.split(';');
        let mime = parts.next().unwrap_or("").trim();
        let mut is_base64 = false;
        let mut raw_w: Option<u32> = None;
        let mut raw_h: Option<u32> = None;
        for param in parts {
            let p = param.trim();
            if p.eq_ignore_ascii_case("base64") {
                is_base64 = true;
            } else if let Some((k, v)) = p.split_once('=') {
                let key = k.trim();
                let val = v.trim();
                if key.eq_ignore_ascii_case("width") || key.eq_ignore_ascii_case("w") {
                    raw_w = val.parse::<u32>().ok();
                } else if key.eq_ignore_ascii_case("height") || key.eq_ignore_ascii_case("h") {
                    raw_h = val.parse::<u32>().ok();
                }
            }
        }
        if !is_base64 {
            return Err(ImageDecodeError::UnsupportedFormat(
                "data URI must be base64-encoded".to_string(),
            ));
        }
        let decoded_bytes = decode_base64(payload)?;
        if mime.eq_ignore_ascii_case("image/png") {
            return decode_png(&decoded_bytes);
        }
        if mime.eq_ignore_ascii_case("image/rgba") {
            let (Some(w), Some(h)) = (raw_w, raw_h) else {
                return Err(ImageDecodeError::InvalidDataUri);
            };
            return DecodedImage::from_rgba(w, h, decoded_bytes);
        }
        return Err(ImageDecodeError::UnsupportedFormat(mime.to_string()));
    }

    if let Some(path_part) = strip_prefix_ascii_ci(trimmed, "file://") {
        let path_str = path_part.strip_prefix("localhost").unwrap_or(path_part);
        let decoded_path = percent_decode(path_str);
        let path = PathBuf::from(decoded_path);
        let bytes = std::fs::read(&path)
            .map_err(|e| ImageDecodeError::Io(format!("{}: {e}", path.display())))?;
        return decode_png(&bytes);
    }

    Err(ImageDecodeError::UnsupportedFormat(trimmed.to_string()))
}

fn strip_prefix_ascii_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    if s.len() < prefix.len() {
        return None;
    }
    let head = s.get(..prefix.len())?;
    if head.eq_ignore_ascii_case(prefix) {
        Some(&s[prefix.len()..])
    } else {
        None
    }
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(hi), Some(lo)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2]))
        {
            out.push((hi << 4) | lo);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[inline]
fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Decodes standard or URL-safe base64 ignoring ASCII whitespace.
pub fn decode_base64(input: &str) -> Result<Vec<u8>, ImageDecodeError> {
    let mut cleaned = Vec::with_capacity(input.len());
    for b in input.bytes() {
        if b.is_ascii_whitespace() {
            continue;
        }
        cleaned.push(b);
    }
    if cleaned.is_empty() {
        return Ok(Vec::new());
    }
    if cleaned.len() % 4 == 1 {
        return Err(ImageDecodeError::InvalidBase64);
    }
    let mut out = Vec::with_capacity((cleaned.len() * 3) / 4);
    let mut i = 0;
    while i < cleaned.len() {
        let rem = cleaned.len() - i;
        let b0 = cleaned[i];
        let b1 = cleaned[i + 1];
        let b2 = if rem > 2 { cleaned[i + 2] } else { b'=' };
        let b3 = if rem > 3 { cleaned[i + 3] } else { b'=' };

        let v0 = b64_val(b0).ok_or(ImageDecodeError::InvalidBase64)?;
        let v1 = b64_val(b1).ok_or(ImageDecodeError::InvalidBase64)?;
        if b2 == b'=' {
            if b3 != b'=' || i + 4 < cleaned.len() {
                return Err(ImageDecodeError::InvalidBase64);
            }
            out.push((v0 << 2) | (v1 >> 4));
            break;
        }
        let v2 = b64_val(b2).ok_or(ImageDecodeError::InvalidBase64)?;
        if b3 == b'=' {
            if i + 4 < cleaned.len() {
                return Err(ImageDecodeError::InvalidBase64);
            }
            out.push((v0 << 2) | (v1 >> 4));
            out.push((v1 << 4) | (v2 >> 2));
            break;
        }
        let v3 = b64_val(b3).ok_or(ImageDecodeError::InvalidBase64)?;
        out.push((v0 << 2) | (v1 >> 4));
        out.push((v1 << 4) | (v2 >> 2));
        out.push((v2 << 6) | v3);
        i += 4;
    }
    Ok(out)
}

#[inline]
fn b64_val(b: u8) -> Option<u8> {
    match b {
        b'A'..=b'Z' => Some(b - b'A'),
        b'a'..=b'z' => Some(b - b'a' + 26),
        b'0'..=b'9' => Some(b - b'0' + 52),
        b'+' | b'-' => Some(62),
        b'/' | b'_' => Some(63),
        _ => None,
    }
}
