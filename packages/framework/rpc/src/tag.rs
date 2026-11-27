//! Zero-allocation typed cache invalidation tags (`Tag64`) and `Nve-Invalidate` HTTP header
//! formatting/parsing with explicit `*` wildcard overflow semantics.

use core::fmt::{self, Write as _};

use nve_hash::Xxh64Hasher;

/// Domain-separated XXH64 seed for cache invalidation tags.
pub const TAG_SEED: u64 = 0x4E56_455F_5441_4731;

/// Standard HTTP response header carrying comma-separated 16-hex invalidated [`Tag64`] hashes
/// or `"*"` when an action invalidates more than [`MAX_INVALIDATE_HEADER_TAGS`] tags.
pub const HEADER_INVALIDATE: &str = "nve-invalidate";

/// Wildcard token emitted when an action invalidates more than [`MAX_INVALIDATE_HEADER_TAGS`].
pub const INVALIDATE_ALL_WILDCARD: &str = "*";

/// Maximum explicit tags emitted/retained in a single `Nve-Invalidate` header before collapsing
/// to [`INVALIDATE_ALL_WILDCARD`].
pub const MAX_INVALIDATE_HEADER_TAGS: usize = 128;

/// Hard cap on comma-separated segments scanned in [`parse_invalidate_header`] to prevent CPU
/// abuse on malformed comma floods.
pub const MAX_INVALIDATE_HEADER_SEGMENTS: usize = MAX_INVALIDATE_HEADER_TAGS * 2;

const TAG_KIND_STATIC: u8 = 0x01;
const TAG_KIND_KEYED_U64: u8 = 0x02;
const TAG_KIND_KEYED_STR: u8 = 0x03;

/// Result of parsing a `Nve-Invalidate` header value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvalidateParse {
    /// Explicit deduplicated tags were appended to `out`.
    Tags,
    /// Server emitted `"*"` (or tag count exceeded [`MAX_INVALIDATE_HEADER_TAGS`]); client
    /// caches should invalidate all entries.
    All,
}

/// 64-bit cache invalidation tag key computed via `xxh64` with zero heap allocation and
/// domain/length separation between static, numeric-keyed, and string-keyed constructors.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Tag64(pub u64);

impl Tag64 {
    /// Reconstructs a [`Tag64`] from a raw 64-bit wire value (e.g. binary hydration snapshots).
    #[inline]
    pub const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    /// Compile-time static tag key (e.g. `Tag64::of("issues.list")`).
    #[inline]
    pub const fn of(name: &str) -> Self {
        let mut h = Xxh64Hasher::new(TAG_SEED);
        h.update(&[TAG_KIND_STATIC]);
        h.update_u64(name.len() as u64);
        h.update(name.as_bytes());
        Self(h.digest())
    }

    /// Zero-allocation numeric-keyed tag (e.g. `Tag64::keyed("issue", issue_id.0)`).
    #[inline]
    pub const fn keyed(prefix: &str, id: u64) -> Self {
        let mut h = Xxh64Hasher::new(TAG_SEED);
        h.update(&[TAG_KIND_KEYED_U64]);
        h.update_u64(prefix.len() as u64);
        h.update(prefix.as_bytes());
        h.update_u64(id);
        Self(h.digest())
    }

    /// Zero-allocation string-keyed tag (e.g. `Tag64::keyed_str("doc", slug)`).
    #[inline]
    pub const fn keyed_str(prefix: &str, key: &str) -> Self {
        let mut h = Xxh64Hasher::new(TAG_SEED);
        h.update(&[TAG_KIND_KEYED_STR]);
        h.update_u64(prefix.len() as u64);
        h.update(prefix.as_bytes());
        h.update_u64(key.len() as u64);
        h.update(key.as_bytes());
        Self(h.digest())
    }

    #[inline]
    pub const fn raw(self) -> u64 {
        self.0
    }

    /// Parses an exact 16-character ASCII hex digest into a [`Tag64`].
    pub fn from_hex(hex: &str) -> Option<Self> {
        let trimmed = hex.trim();
        if trimmed.len() != 16 {
            return None;
        }
        let mut val = 0u64;
        for b in trimmed.bytes() {
            let nibble = match b {
                b'0'..=b'9' => (b - b'0') as u64,
                b'a'..=b'f' => (b - b'a' + 10) as u64,
                b'A'..=b'F' => (b - b'A' + 10) as u64,
                _ => return None,
            };
            val = (val << 4) | nibble;
        }
        Some(Self(val))
    }
}

impl fmt::Display for Tag64 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:016x}", self.0)
    }
}

/// Implemented by application tag enums to convert strongly typed tags into [`Tag64`] without
/// heap allocations.
pub trait CacheTag {
    fn tag64(&self) -> Tag64;
}

impl CacheTag for Tag64 {
    #[inline]
    fn tag64(&self) -> Tag64 {
        *self
    }
}

/// Inline-backed tag sink avoiding heap allocation for up to 8 tags per request and latching
/// into O(1) wildcard overflow (`*`) once unique tags exceed [`MAX_INVALIDATE_HEADER_TAGS`].
#[derive(Debug, Clone, Default)]
pub struct TagSink {
    inline: [Tag64; 8],
    inline_len: u8,
    overflow_all: bool,
    overflow: Option<Vec<Tag64>>,
}

impl TagSink {
    #[inline]
    pub const fn new() -> Self {
        Self {
            inline: [Tag64(0); 8],
            inline_len: 0,
            overflow_all: false,
            overflow: None,
        }
    }

    /// Latches this sink into wildcard invalidate-all (`*`) mode and frees any stored tags.
    #[inline]
    pub fn mark_all(&mut self) {
        self.overflow_all = true;
        self.inline_len = 0;
        self.overflow = None;
    }

    /// Returns `true` when this sink latched into wildcard invalidate-all (`*`) mode.
    #[inline]
    pub const fn is_overflow_all(&self) -> bool {
        self.overflow_all
    }

    #[inline]
    pub fn push(&mut self, tag: impl CacheTag) {
        if self.overflow_all {
            return;
        }
        let t = tag.tag64();
        if self.contains(t) {
            return;
        }
        if self.len() >= MAX_INVALIDATE_HEADER_TAGS {
            self.mark_all();
            return;
        }
        if let Some(vec) = &mut self.overflow {
            vec.push(t);
            return;
        }
        let len = self.inline_len as usize;
        if len < self.inline.len() {
            self.inline[len] = t;
            self.inline_len += 1;
        } else {
            let mut vec = Vec::with_capacity(16);
            vec.extend_from_slice(&self.inline);
            vec.push(t);
            self.overflow = Some(vec);
        }
    }

    #[inline]
    pub fn extend_tags<T: CacheTag>(&mut self, tags: impl IntoIterator<Item = T>) {
        for tag in tags {
            if self.overflow_all {
                break;
            }
            self.push(tag);
        }
    }

    #[inline]
    pub fn as_slice(&self) -> &[Tag64] {
        match &self.overflow {
            Some(vec) => vec.as_slice(),
            None => &self.inline[..self.inline_len as usize],
        }
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        !self.overflow_all && self.as_slice().is_empty()
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.as_slice().len()
    }

    #[inline]
    pub fn contains(&self, tag: Tag64) -> bool {
        self.as_slice().contains(&tag)
    }

    #[inline]
    pub fn clear(&mut self) {
        self.inline_len = 0;
        self.overflow_all = false;
        if let Some(vec) = &mut self.overflow {
            vec.clear();
        }
    }
}

/// Formats a [`TagSink`] as a `Nve-Invalidate` header value, emitting `"*"` when latched into
/// wildcard overflow and `None` when empty.
pub fn format_invalidate_sink(sink: &TagSink) -> Option<String> {
    if sink.is_overflow_all() {
        return Some(INVALIDATE_ALL_WILDCARD.to_owned());
    }
    format_invalidate_header(sink.as_slice())
}

/// Formats `tags` as a comma-separated lowercase 16-hex `Nve-Invalidate` header value into `out`.
///
/// When `tags.len() > MAX_INVALIDATE_HEADER_TAGS`, writes [`INVALIDATE_ALL_WILDCARD`] (`"*"`) so
/// clients invalidate cleanly instead of truncating tags.
pub fn format_invalidate_header_into(tags: &[Tag64], out: &mut String) {
    out.clear();
    if tags.len() > MAX_INVALIDATE_HEADER_TAGS {
        out.push_str(INVALIDATE_ALL_WILDCARD);
        return;
    }
    for (idx, tag) in tags.iter().enumerate() {
        if idx > 0 {
            out.push(',');
        }
        let _ = write!(out, "{:016x}", tag.0);
    }
}

/// Formats `tags` as a `Nve-Invalidate` header value, returning `None` when `tags` is empty and
/// `"*"` when `tags.len() > MAX_INVALIDATE_HEADER_TAGS`.
pub fn format_invalidate_header(tags: &[Tag64]) -> Option<String> {
    if tags.is_empty() {
        return None;
    }
    if tags.len() > MAX_INVALIDATE_HEADER_TAGS {
        return Some(INVALIDATE_ALL_WILDCARD.to_owned());
    }
    let mut out = String::with_capacity(tags.len() * 17);
    format_invalidate_header_into(tags, &mut out);
    Some(out)
}

/// Parses a `Nve-Invalidate` header value into `out`, collapsing `"*"` or tag counts exceeding
/// [`MAX_INVALIDATE_HEADER_TAGS`] into [`InvalidateParse::All`] and rejecting malformed syntax or
/// excessive comma floods.
pub fn parse_invalidate_header(
    raw: &str,
    out: &mut Vec<Tag64>,
) -> Result<InvalidateParse, &'static str> {
    let mut segments = 0usize;
    for part in raw.split(',') {
        segments += 1;
        if segments > MAX_INVALIDATE_HEADER_SEGMENTS {
            return Err("too many segments in Nve-Invalidate header");
        }
        let trimmed = part.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed == INVALIDATE_ALL_WILDCARD {
            out.clear();
            return Ok(InvalidateParse::All);
        }
        let tag = Tag64::from_hex(trimmed).ok_or("invalid hex tag in Nve-Invalidate header")?;
        if !out.contains(&tag) {
            if out.len() >= MAX_INVALIDATE_HEADER_TAGS {
                out.clear();
                return Ok(InvalidateParse::All);
            }
            out.push(tag);
        }
    }
    Ok(InvalidateParse::Tags)
}
