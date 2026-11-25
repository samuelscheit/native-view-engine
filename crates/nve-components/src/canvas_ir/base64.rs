//! Zero-dependency strict RFC 4648 base64 codec for UTF-8 RSX `drawList` attributes.

use super::types::DrawListError;

const B64_ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub(super) fn encode_base64_into(input: &[u8], out: &mut String) {
    let mut chunks = input.chunks_exact(3);
    for chunk in &mut chunks {
        let n = ((chunk[0] as u32) << 16) | ((chunk[1] as u32) << 8) | (chunk[2] as u32);
        out.push(B64_ALPHABET[((n >> 18) & 0x3f) as usize] as char);
        out.push(B64_ALPHABET[((n >> 12) & 0x3f) as usize] as char);
        out.push(B64_ALPHABET[((n >> 6) & 0x3f) as usize] as char);
        out.push(B64_ALPHABET[(n & 0x3f) as usize] as char);
    }
    let rem = chunks.remainder();
    match rem.len() {
        1 => {
            let n = (rem[0] as u32) << 16;
            out.push(B64_ALPHABET[((n >> 18) & 0x3f) as usize] as char);
            out.push(B64_ALPHABET[((n >> 12) & 0x3f) as usize] as char);
            out.push('=');
            out.push('=');
        }
        2 => {
            let n = ((rem[0] as u32) << 16) | ((rem[1] as u32) << 8);
            out.push(B64_ALPHABET[((n >> 18) & 0x3f) as usize] as char);
            out.push(B64_ALPHABET[((n >> 12) & 0x3f) as usize] as char);
            out.push(B64_ALPHABET[((n >> 6) & 0x3f) as usize] as char);
            out.push('=');
        }
        _ => {}
    }
}

fn decode_b64_digit(b: u8) -> Option<u8> {
    match b {
        b'A'..=b'Z' => Some(b - b'A'),
        b'a'..=b'z' => Some(b - b'a' + 26),
        b'0'..=b'9' => Some(b - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

pub(super) fn decode_base64(input: &str) -> Result<Vec<u8>, DrawListError> {
    let raw = input.as_bytes();
    if raw.is_empty() || !raw.len().is_multiple_of(4) {
        return Err(DrawListError::InvalidBase64);
    }
    let mut out = Vec::with_capacity((raw.len() / 4) * 3);
    for (idx, chunk) in raw.chunks_exact(4).enumerate() {
        let is_last = (idx + 1) * 4 == raw.len();
        let a = decode_b64_digit(chunk[0]).ok_or(DrawListError::InvalidBase64)?;
        let b = decode_b64_digit(chunk[1]).ok_or(DrawListError::InvalidBase64)?;
        if chunk[2] == b'=' {
            if !is_last || chunk[3] != b'=' {
                return Err(DrawListError::InvalidBase64);
            }
            out.push((a << 2) | (b >> 4));
        } else {
            let c = decode_b64_digit(chunk[2]).ok_or(DrawListError::InvalidBase64)?;
            out.push((a << 2) | (b >> 4));
            if chunk[3] == b'=' {
                if !is_last {
                    return Err(DrawListError::InvalidBase64);
                }
                out.push((b << 4) | (c >> 2));
            } else {
                let d = decode_b64_digit(chunk[3]).ok_or(DrawListError::InvalidBase64)?;
                out.push((b << 4) | (c >> 2));
                out.push((c << 6) | d);
            }
        }
    }
    Ok(out)
}
