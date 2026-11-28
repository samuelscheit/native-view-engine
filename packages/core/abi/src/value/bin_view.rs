use std::marker::PhantomData;

use crate::DecodeError;

use super::bin_codec::{read_u32_le, skip_validated_value, validate_value_at};
use super::ffi_types::NveValueTag;

/// Zero-copy, zero-allocation borrowed reader over a validated binary `NveBinValue` byte slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BinValueView<'a> {
    bytes: &'a [u8],
    tag: NveValueTag,
}

impl<'a> BinValueView<'a> {
    /// Validates `bytes` once (zero heap allocation) and returns a borrowed `BinValueView<'a>`.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, DecodeError> {
        let (tag, end) = validate_value_at(bytes, 0, 0)?;
        if end != bytes.len() {
            return Err(DecodeError::TrailingBytes(bytes.len() - end));
        }
        Ok(Self { bytes, tag })
    }

    /// View over bytes a `parse` already accepted (an owner that stores validated bytes).
    #[inline]
    pub(crate) fn from_validated(bytes: &'a [u8]) -> Self {
        let tag = bytes
            .first()
            .and_then(|&raw| NveValueTag::try_from_u32(u32::from(raw)))
            .unwrap_or(NveValueTag::Null);
        Self { bytes, tag }
    }

    #[inline]
    pub const fn tag(&self) -> NveValueTag {
        self.tag
    }

    #[inline]
    pub const fn as_raw_bytes(&self) -> &'a [u8] {
        self.bytes
    }

    #[inline]
    pub const fn is_null(&self) -> bool {
        matches!(self.tag, NveValueTag::Null)
    }

    #[inline]
    pub fn as_bool(&self) -> Option<bool> {
        (self.tag == NveValueTag::Bool).then(|| self.bytes[1] != 0)
    }

    #[inline]
    pub fn as_i64(&self) -> Option<i64> {
        if self.tag != NveValueTag::I64 {
            return None;
        }
        let b: [u8; 8] = self.bytes[1..9].try_into().ok()?;
        Some(i64::from_le_bytes(b))
    }

    #[inline]
    pub fn as_f64(&self) -> Option<f64> {
        if self.tag != NveValueTag::F64 {
            return None;
        }
        let b: [u8; 8] = self.bytes[1..9].try_into().ok()?;
        Some(f64::from_le_bytes(b))
    }

    #[inline]
    pub fn as_number(&self) -> Option<f64> {
        match self.tag {
            NveValueTag::F64 => self.as_f64(),
            NveValueTag::I64 => self.as_i64().map(|v| v as f64),
            _ => None,
        }
    }

    #[inline]
    pub fn as_str(&self) -> Option<&'a str> {
        if self.tag != NveValueTag::Str {
            return None;
        }
        let len = read_u32_le(self.bytes, 1).ok()? as usize;
        std::str::from_utf8(self.bytes.get(5..5 + len)?).ok()
    }

    #[inline]
    pub fn as_bytes(&self) -> Option<&'a [u8]> {
        if self.tag != NveValueTag::Bytes {
            return None;
        }
        let len = read_u32_le(self.bytes, 1).ok()? as usize;
        self.bytes.get(5..5 + len)
    }

    #[inline]
    pub fn array_len(&self) -> Option<usize> {
        if self.tag != NveValueTag::Array {
            return None;
        }
        Some(read_u32_le(self.bytes, 5).ok()? as usize)
    }

    /// O(index) single-pass skip to element `index` (nested containers skipped in O(1)).
    pub fn array_at(&self, index: usize) -> Option<BinValueView<'a>> {
        let count = self.array_len()?;
        if index >= count {
            return None;
        }
        let mut pos = 9usize;
        for i in 0..=index {
            let (tag, end) = skip_validated_value(self.bytes, pos)?;
            if i == index {
                return Some(BinValueView {
                    bytes: &self.bytes[pos..end],
                    tag,
                });
            }
            pos = end;
        }
        None
    }

    #[inline]
    pub fn as_array(&self) -> Option<BinArrayIter<'a>> {
        let remaining = self.array_len()?;
        Some(BinArrayIter {
            bytes: self.bytes,
            pos: 9,
            remaining,
            _marker: PhantomData,
        })
    }

    #[inline]
    pub fn map_len(&self) -> Option<usize> {
        if self.tag != NveValueTag::Map {
            return None;
        }
        Some(read_u32_le(self.bytes, 5).ok()? as usize)
    }

    /// Zero-copy borrowed key lookup on a binary map (nested values skipped in O(1)).
    pub fn get(&self, key: &str) -> Option<BinValueView<'a>> {
        let count = self.map_len()?;
        let target = key.as_bytes();
        let mut pos = 9usize;
        for _ in 0..count {
            let klen = read_u32_le(self.bytes, pos).ok()? as usize;
            pos += 4;
            let kbytes = self.bytes.get(pos..pos + klen)?;
            pos += klen;
            let (vtag, vend) = skip_validated_value(self.bytes, pos)?;
            if kbytes == target {
                return Some(BinValueView {
                    bytes: &self.bytes[pos..vend],
                    tag: vtag,
                });
            }
            pos = vend;
        }
        None
    }

    #[inline]
    pub fn map_get(&self, key: &str) -> Option<BinValueView<'a>> {
        self.get(key)
    }

    #[inline]
    pub fn as_map(&self) -> Option<BinMapIter<'a>> {
        let remaining = self.map_len()?;
        Some(BinMapIter {
            bytes: self.bytes,
            pos: 9,
            remaining,
            _marker: PhantomData,
        })
    }
}

pub struct BinArrayIter<'a> {
    bytes: &'a [u8],
    pos: usize,
    remaining: usize,
    _marker: PhantomData<&'a [u8]>,
}

impl<'a> Iterator for BinArrayIter<'a> {
    type Item = BinValueView<'a>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        let (tag, end) = skip_validated_value(self.bytes, self.pos)?;
        let item = BinValueView {
            bytes: &self.bytes[self.pos..end],
            tag,
        };
        self.pos = end;
        self.remaining -= 1;
        Some(item)
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl ExactSizeIterator for BinArrayIter<'_> {}

pub struct BinMapIter<'a> {
    bytes: &'a [u8],
    pos: usize,
    remaining: usize,
    _marker: PhantomData<&'a [u8]>,
}

impl<'a> Iterator for BinMapIter<'a> {
    type Item = (&'a str, BinValueView<'a>);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        let klen = read_u32_le(self.bytes, self.pos).ok()? as usize;
        let kstart = self.pos + 4;
        let kend = kstart + klen;
        let key = std::str::from_utf8(self.bytes.get(kstart..kend)?).ok()?;
        let (tag, vend) = skip_validated_value(self.bytes, kend)?;
        let val = BinValueView {
            bytes: &self.bytes[kend..vend],
            tag,
        };
        self.pos = vend;
        self.remaining -= 1;
        Some((key, val))
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl ExactSizeIterator for BinMapIter<'_> {}
