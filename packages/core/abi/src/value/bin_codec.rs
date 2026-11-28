// Compact Binary `NveBinValue` Wire Encoder & Validator
//
// Wire layout (all integers little-endian):
//   tag: u8 (0=Null, 1=Bool, 2=I64, 3=F64, 4=Str, 5=Bytes, 6=Array, 7=Map)
//   - Null:  [0]
//   - Bool:  [1, b: u8 (0|1)]
//   - I64:   [2, v: i64_le]
//   - F64:   [3, v: f64_le]
//   - Str:   [4, len: u32_le, utf8_bytes[len]]
//   - Bytes: [5, len: u32_le, raw_bytes[len]]
//   - Array: [6, payload_byte_len: u32_le, count: u32_le, elem_0, ..., elem_{count-1}]
//   - Map:   [7, payload_byte_len: u32_le, count: u32_le, (klen: u32_le, key_utf8[klen], val), ...]
//
// `payload_byte_len` covers everything AFTER the `payload_byte_len` field (i.e. `4 + children_bytes`),
// enabling O(1) subtree skipping during borrowed lookups.

use crate::DecodeError;

use super::bin_writer::BinWriter;
use super::ffi_types::NveValueTag;
use super::ffi_views::NveValueRef;
use super::NVE_BIN_VALUE_MAX_DEPTH;

/// Encodes an `NveValueRef` into `w` (errors are recorded in `w`, see [`BinWriter::finish`]).
pub fn encode_bin_value(value: NveValueRef<'_>, w: &mut BinWriter) {
    let Some(tag) = value.tag() else {
        return w.fail(DecodeError::InvalidEnum("NveValueTag", value.raw.tag));
    };
    match tag {
        NveValueTag::Null => w.null(),
        NveValueTag::Bool => w.bool(value.as_bool().unwrap_or_default()),
        NveValueTag::I64 => w.i64(value.as_i64().unwrap_or_default()),
        NveValueTag::F64 => w.f64(value.as_f64().unwrap_or_default()),
        // SAFETY: `tag` is `Str` and `NveValueRef` guarantees valid nested pointers.
        NveValueTag::Str => match unsafe { value.raw.data.str_val.as_str() } {
            Ok(s) => w.str(s),
            Err(e) => w.fail(e),
        },
        NveValueTag::Bytes => match value.as_bytes() {
            Some(b) => w.bytes(b),
            None => w.fail(DecodeError::Truncated),
        },
        NveValueTag::Array => match value.as_array() {
            Some(items) => w.array(|a| items.iter().for_each(|item| encode_bin_value(item, a.item()))),
            None => w.fail(DecodeError::Truncated),
        },
        NveValueTag::Map => match value.as_map() {
            Some(map) => w.map(|m| {
                for entry in map.entries {
                    // SAFETY: `NveMapRef` guarantees valid immutable entry key pointers for its lifetime.
                    match unsafe { entry.key.as_str() } {
                        Ok(key) => encode_bin_value(NveValueRef { raw: &entry.value }, m.key(key)),
                        Err(e) => m.key("").fail(e),
                    }
                }
            }),
            None => w.fail(DecodeError::Truncated),
        },
    }
}

/// Encodes `value` into a new binary `NveBinValue` buffer.
pub fn encode_bin_value_to_vec(value: NveValueRef<'_>) -> Result<Vec<u8>, DecodeError> {
    let mut w = BinWriter::with_capacity(64);
    encode_bin_value(value, &mut w);
    w.finish()
}

#[inline]
pub(super) fn read_u32_le(bytes: &[u8], pos: usize) -> Result<u32, DecodeError> {
    let end = pos.checked_add(4).ok_or(DecodeError::Truncated)?;
    let slice = bytes.get(pos..end).ok_or(DecodeError::Truncated)?;
    Ok(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

/// Returns the byte length of one encoded value starting at `bytes[start..]`, validating structure,
/// UTF-8, bool range, bounds, and nesting depth (`<= NVE_BIN_VALUE_MAX_DEPTH`).
pub(super) fn validate_value_at(
    bytes: &[u8],
    start: usize,
    depth: usize,
) -> Result<(NveValueTag, usize), DecodeError> {
    let raw_tag = *bytes.get(start).ok_or(DecodeError::Truncated)?;
    let tag = NveValueTag::try_from_u32(u32::from(raw_tag))
        .ok_or(DecodeError::InvalidEnum("NveValueTag", u32::from(raw_tag)))?;
    let mut pos = start + 1;
    match tag {
        NveValueTag::Null => Ok((tag, pos)),
        NveValueTag::Bool => {
            let b = *bytes.get(pos).ok_or(DecodeError::Truncated)?;
            if b > 1 {
                return Err(DecodeError::InvalidEnum("NveBinBool", b as u32));
            }
            Ok((tag, pos + 1))
        }
        NveValueTag::I64 | NveValueTag::F64 => {
            let end = pos.checked_add(8).ok_or(DecodeError::Truncated)?;
            if end > bytes.len() {
                return Err(DecodeError::Truncated);
            }
            Ok((tag, end))
        }
        NveValueTag::Str => {
            let len = read_u32_le(bytes, pos)? as usize;
            pos += 4;
            let end = pos.checked_add(len).ok_or(DecodeError::Truncated)?;
            let slice = bytes.get(pos..end).ok_or(DecodeError::Truncated)?;
            std::str::from_utf8(slice).map_err(|_| DecodeError::InvalidUtf8)?;
            Ok((tag, end))
        }
        NveValueTag::Bytes => {
            let len = read_u32_le(bytes, pos)? as usize;
            pos += 4;
            let end = pos.checked_add(len).ok_or(DecodeError::Truncated)?;
            if end > bytes.len() {
                return Err(DecodeError::Truncated);
            }
            Ok((tag, end))
        }
        NveValueTag::Array => {
            if depth >= NVE_BIN_VALUE_MAX_DEPTH {
                return Err(DecodeError::MaxDepthExceeded);
            }
            let payload_len = read_u32_le(bytes, pos)? as usize;
            pos += 4;
            let container_end = pos.checked_add(payload_len).ok_or(DecodeError::Truncated)?;
            if container_end > bytes.len() || payload_len < 4 {
                return Err(DecodeError::Truncated);
            }
            let count = read_u32_le(bytes, pos)? as usize;
            pos += 4;
            // Each element occupies at least 1 byte.
            if count > container_end - pos {
                return Err(DecodeError::Truncated);
            }
            let bounded = &bytes[..container_end];
            for _ in 0..count {
                let (_, next_pos) = validate_value_at(bounded, pos, depth + 1)?;
                pos = next_pos;
            }
            if pos != container_end {
                return Err(DecodeError::TrailingBytes(container_end - pos));
            }
            Ok((tag, container_end))
        }
        NveValueTag::Map => {
            if depth >= NVE_BIN_VALUE_MAX_DEPTH {
                return Err(DecodeError::MaxDepthExceeded);
            }
            let payload_len = read_u32_le(bytes, pos)? as usize;
            pos += 4;
            let container_end = pos.checked_add(payload_len).ok_or(DecodeError::Truncated)?;
            if container_end > bytes.len() || payload_len < 4 {
                return Err(DecodeError::Truncated);
            }
            let count = read_u32_le(bytes, pos)? as usize;
            pos += 4;
            // Each map entry occupies at least 5 bytes (u32 key_len + 1-byte value).
            if count > (container_end - pos) / 5 {
                return Err(DecodeError::Truncated);
            }
            let bounded = &bytes[..container_end];
            for _ in 0..count {
                let klen = read_u32_le(bounded, pos)? as usize;
                pos += 4;
                let kend = pos.checked_add(klen).ok_or(DecodeError::Truncated)?;
                let kslice = bounded.get(pos..kend).ok_or(DecodeError::Truncated)?;
                std::str::from_utf8(kslice).map_err(|_| DecodeError::InvalidUtf8)?;
                pos = kend;
                let (_, next_pos) = validate_value_at(bounded, pos, depth + 1)?;
                pos = next_pos;
            }
            if pos != container_end {
                return Err(DecodeError::TrailingBytes(container_end - pos));
            }
            Ok((tag, container_end))
        }
    }
}

/// Fast O(1) skip over one already-validated value starting at `bytes[start..]`.
#[inline]
pub(super) fn skip_validated_value(bytes: &[u8], start: usize) -> Option<(NveValueTag, usize)> {
    let tag = NveValueTag::try_from_u32(u32::from(*bytes.get(start)?))?;
    let pos = start + 1;
    let end = match tag {
        NveValueTag::Null => pos,
        NveValueTag::Bool => pos + 1,
        NveValueTag::I64 | NveValueTag::F64 => pos + 8,
        NveValueTag::Str | NveValueTag::Bytes | NveValueTag::Array | NveValueTag::Map => {
            let len = read_u32_le(bytes, pos).ok()? as usize;
            pos.checked_add(4)?.checked_add(len)?
        }
    };
    (end <= bytes.len()).then_some((tag, end))
}
