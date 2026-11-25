//! UAX #29 extended grapheme cluster, word-boundary, paragraph-boundary, and line-ending
//! normalization primitives shared by `nve-backend-skia` and `nve-skia-shell`.

use unicode_segmentation::UnicodeSegmentation;

/// Clamps `byte_index` to the nearest UAX #29 extended grapheme cluster boundary at or before
/// `byte_index` (never splits ZWJ emoji, regional flag pairs, skin-tone modifiers, or combining
/// diacritics).
#[inline]
pub fn clamp_grapheme_boundary(s: &str, byte_index: usize) -> usize {
    if s.is_empty() || byte_index == 0 {
        return 0;
    }
    if byte_index >= s.len() {
        return s.len();
    }
    let mut last = 0usize;
    for (idx, _) in s.grapheme_indices(true) {
        if idx > byte_index {
            break;
        }
        last = idx;
    }
    last
}

/// Returns `true` if `byte_index` lies on a valid UAX #29 extended grapheme cluster boundary
/// (including `0` and `s.len()`).
#[inline]
pub fn is_grapheme_boundary(s: &str, byte_index: usize) -> bool {
    byte_index <= s.len() && clamp_grapheme_boundary(s, byte_index) == byte_index
}

/// Steps backward by one UAX #29 extended grapheme cluster from `byte_index`.
#[inline]
pub fn prev_grapheme_boundary(s: &str, byte_index: usize) -> usize {
    let idx = clamp_grapheme_boundary(s, byte_index);
    if idx == 0 {
        return 0;
    }
    let mut prev = 0usize;
    for (g_idx, _) in s.grapheme_indices(true) {
        if g_idx >= idx {
            break;
        }
        prev = g_idx;
    }
    prev
}

/// Steps forward by one UAX #29 extended grapheme cluster from `byte_index`.
#[inline]
pub fn next_grapheme_boundary(s: &str, byte_index: usize) -> usize {
    let idx = clamp_grapheme_boundary(s, byte_index);
    if idx >= s.len() {
        return s.len();
    }
    for (g_idx, g) in s.grapheme_indices(true) {
        if g_idx == idx {
            return g_idx + g.len();
        }
    }
    s.len()
}

/// Returns every UAX #29 extended grapheme cluster boundary in `s`, always starting at `0`
/// and ending at `s.len()`.
pub fn grapheme_boundaries(s: &str) -> Vec<usize> {
    let mut out: Vec<usize> = s.grapheme_indices(true).map(|(i, _)| i).collect();
    out.push(s.len());
    out
}

/// Returns the number of UAX #29 extended grapheme clusters in `s`.
#[inline]
pub fn grapheme_count(s: &str) -> usize {
    s.graphemes(true).count()
}

/// Computes the `(start, end)` UTF-8 byte range of the UAX #29 word (or contiguous whitespace/
/// punctuation/emoji cluster) surrounding `byte_index` for double-click selection.
/// Both returned offsets are guaranteed to lie on valid grapheme cluster boundaries.
pub fn word_bounds_at(s: &str, byte_index: usize) -> (usize, usize) {
    if s.is_empty() {
        return (0, 0);
    }
    let mut idx = clamp_grapheme_boundary(s, byte_index);
    if idx == s.len() {
        idx = prev_grapheme_boundary(s, idx);
    }

    for (seg_start, seg) in s.split_word_bound_indices() {
        let seg_end = seg_start + seg.len();
        if idx >= seg_start && idx < seg_end {
            let start = clamp_grapheme_boundary(s, seg_start);
            let mut end = clamp_grapheme_boundary(s, seg_end);
            if end < seg_end {
                end = next_grapheme_boundary(s, end);
            }
            return (start, end.max(start));
        }
    }

    let next = next_grapheme_boundary(s, idx);
    (idx, next)
}

/// Computes the `(start, end)` UTF-8 byte range of the line/paragraph containing `byte_index`
/// for triple-click selection. Excludes trailing `\r\n` / `\n` / `\r` line breaks when selecting
/// within a multi-line string, and selects `(0, s.len())` for single-line text.
pub fn paragraph_bounds_at(s: &str, byte_index: usize) -> (usize, usize) {
    if s.is_empty() {
        return (0, 0);
    }
    let mut idx = clamp_grapheme_boundary(s, byte_index);
    if idx == s.len() && idx > 0 {
        let prev = prev_grapheme_boundary(s, idx);
        let tail = &s[prev..idx];
        if tail != "\n" && tail != "\r\n" && tail != "\r" {
            idx = prev;
        }
    }

    let bytes = s.as_bytes();
    let mut start = idx;
    while start > 0 {
        let b = bytes[start - 1];
        if b == b'\n' || b == b'\r' {
            break;
        }
        start -= 1;
    }

    let mut end = idx;
    while end < bytes.len() {
        let b = bytes[end];
        if b == b'\n' || b == b'\r' {
            break;
        }
        end += 1;
    }

    let start = clamp_grapheme_boundary(s, start);
    let end = clamp_grapheme_boundary(s, end).max(start);
    (start, end)
}

/// Normalizes CRLF (`\r\n`) and legacy CR (`\r`) line endings into Unix LF (`\n`).
pub fn normalize_line_endings(raw: &str) -> String {
    if !raw.contains('\r') {
        return raw.to_owned();
    }
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\r' {
            if chars.peek() == Some(&'\n') {
                chars.next();
            }
            out.push('\n');
        } else {
            out.push(ch);
        }
    }
    out
}
