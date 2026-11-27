//! Safe 4-byte-aligned byte-slice and UTF-8 string encoding/validation/reading helpers for
//! [`crate::FlatResource`] implementations.
//!
//! Each byte/string block occupies `1 + byte_len.div_ceil(4)` `u32` words:
//! `[byte_len: u32, payload_words...]`, with trailing padding bytes in the final word required
//! to be zero so identical content hashes deterministically under `xxh64`.

use crate::resource::words_as_le_bytes;

/// Number of `u32` payload words required to hold `byte_len` bytes (rounded up to 4 bytes).
#[inline(always)]
pub const fn padded_word_len(byte_len: usize) -> usize {
    byte_len.div_ceil(4)
}

/// Appends `[byte_len: u32, zero_padded_le_words...]` to `out`, returning `false` if `bytes.len()`
/// exceeds `max_bytes` or `u32::MAX` (leaving `out` unchanged on failure).
#[inline]
pub fn write_bytes_words(out: &mut Vec<u32>, bytes: &[u8], max_bytes: usize) -> bool {
    let len = bytes.len();
    if len > max_bytes || len > u32::MAX as usize {
        return false;
    }
    let word_count = padded_word_len(len);
    out.reserve(1 + word_count);
    out.push(len as u32);
    let (chunks, rem) = bytes.as_chunks::<4>();
    for &c in chunks {
        out.push(u32::from_le_bytes(c));
    }
    if !rem.is_empty() {
        let mut tail = [0u8; 4];
        tail[..rem.len()].copy_from_slice(rem);
        out.push(u32::from_le_bytes(tail));
    }
    true
}

/// Appends `[byte_len: u32, zero_padded_utf8_words...]` to `out`.
#[inline]
pub fn write_str_words(out: &mut Vec<u32>, text: &str, max_bytes: usize) -> bool {
    write_bytes_words(out, text.as_bytes(), max_bytes)
}

/// Validates a length-prefixed byte block starting at `words[*pos]` (enforcing `len <= max_bytes`,
/// sufficient words, and zeroed trailing pad bytes) and advances `*pos` past it.
#[inline]
pub fn validate_bytes_words(words: &[u32], pos: &mut usize, max_bytes: usize) -> bool {
    let Some(&len_u32) = words.get(*pos) else {
        return false;
    };
    let len = len_u32 as usize;
    if len > max_bytes {
        return false;
    }
    let wc = padded_word_len(len);
    let Some(start) = pos.checked_add(1) else {
        return false;
    };
    let Some(end) = start.checked_add(wc) else {
        return false;
    };
    if end > words.len() {
        return false;
    }
    let rem = len & 3;
    if rem != 0 {
        let last_word = words[end - 1];
        let pad_mask = !((1u32 << (rem * 8)) - 1);
        if (last_word & pad_mask) != 0 {
            return false;
        }
    }
    *pos = end;
    true
}

/// Validates a length-prefixed UTF-8 string block starting at `words[*pos]` (enforcing
/// `len <= max_bytes`, zeroed trailing pad bytes, and valid UTF-8) and advances `*pos` past it.
#[inline]
pub fn validate_str_words(words: &[u32], pos: &mut usize, max_bytes: usize) -> bool {
    let start = *pos;
    if !validate_bytes_words(words, pos, max_bytes) {
        return false;
    }
    let len = words[start] as usize;
    let payload = &words[start + 1..*pos];
    let raw = &words_as_le_bytes(payload)[..len];
    core::str::from_utf8(raw).is_ok()
}

/// Reads a length-prefixed byte slice starting at `words[*pos]` and advances `*pos` past it
/// (100% safe; returns `None` on out-of-bounds).
#[inline]
pub fn read_bytes_words<'a>(words: &'a [u32], pos: &mut usize) -> Option<&'a [u8]> {
    let len = *words.get(*pos)? as usize;
    let wc = padded_word_len(len);
    let start = pos.checked_add(1)?;
    let end = start.checked_add(wc)?;
    let slice = words.get(start..end)?;
    *pos = end;
    words_as_le_bytes(slice).get(..len)
}

/// Reads a length-prefixed UTF-8 `&str` starting at `words[*pos]` and advances `*pos` past it
/// (100% safe, zero `unsafe`; returns `None` on out-of-bounds or invalid UTF-8).
#[inline]
pub fn read_str_words<'a>(words: &'a [u32], pos: &mut usize) -> Option<&'a str> {
    let bytes = read_bytes_words(words, pos)?;
    core::str::from_utf8(bytes).ok()
}

/// Validates `n` concatenated UTF-8 strings packed as `[len_0, ..., len_{n-1}, bytes_padded...]`.
#[inline]
pub fn validate_fixed_str_pack<const N: usize>(words: &[u32], max_per_str: usize) -> bool {
    if words.len() < N {
        return false;
    }
    let mut total_bytes = 0usize;
    for &l in &words[..N] {
        let lu = l as usize;
        if lu > max_per_str {
            return false;
        }
        let Some(next) = total_bytes.checked_add(lu) else {
            return false;
        };
        total_bytes = next;
    }
    let payload_words = padded_word_len(total_bytes);
    if words.len() != N + payload_words {
        return false;
    }
    let rem = total_bytes & 3;
    if rem != 0 {
        let last_word = words[N + payload_words - 1];
        let pad_mask = !((1u32 << (rem * 8)) - 1);
        if (last_word & pad_mask) != 0 {
            return false;
        }
    }
    let raw = &words_as_le_bytes(&words[N..])[..total_bytes];
    let mut off = 0usize;
    for &l in &words[..N] {
        let end = off + l as usize;
        if core::str::from_utf8(&raw[off..end]).is_err() {
            return false;
        }
        off = end;
    }
    true
}

/// Reads string `index` (`0..N`) from a `[len_0, ..., len_{N-1}, bytes_padded...]` pack
/// (100% safe; returns `""` on out-of-bounds or invalid UTF-8).
#[inline]
pub fn read_fixed_str_pack<const N: usize>(words: &[u32], index: usize) -> &str {
    if index >= N || words.len() < N {
        return "";
    }
    let mut off = 0usize;
    for &l in &words[..index] {
        off = off.saturating_add(l as usize);
    }
    let len = words[index] as usize;
    let Some(end) = off.checked_add(len) else {
        return "";
    };
    let bytes = words_as_le_bytes(&words[N..]);
    bytes
        .get(off..end)
        .and_then(|s| core::str::from_utf8(s).ok())
        .unwrap_or("")
}

/// Writes `N` strings into a compact `[len_0, ..., len_{N-1}, bytes_padded...]` pack.
pub fn write_fixed_str_pack<const N: usize>(
    out: &mut Vec<u32>,
    strs: [&str; N],
    max_per_str: usize,
) -> bool {
    let mut total_bytes = 0usize;
    for s in strs {
        if s.len() > max_per_str || s.len() > u32::MAX as usize {
            return false;
        }
        let Some(next) = total_bytes.checked_add(s.len()) else {
            return false;
        };
        total_bytes = next;
    }
    let payload_words = padded_word_len(total_bytes);
    out.reserve(N + payload_words);
    for s in strs {
        out.push(s.len() as u32);
    }
    let mut word_bytes = [0u8; 4];
    let mut fill = 0usize;
    for s in strs {
        for &b in s.as_bytes() {
            word_bytes[fill] = b;
            fill += 1;
            if fill == 4 {
                out.push(u32::from_le_bytes(word_bytes));
                word_bytes = [0u8; 4];
                fill = 0;
            }
        }
    }
    if fill != 0 {
        out.push(u32::from_le_bytes(word_bytes));
    }
    true
}
