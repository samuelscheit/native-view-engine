//! Strictly bounds-checked little-endian binary reader.

use crate::{canonical_f32_bits, canonical_f64_bits, CodecError, NveDecode};

/// Strictly bounds-checked little-endian binary reader.
#[derive(Debug, Clone)]
pub struct ByteReader<'a> {
    data: &'a [u8],
    pos: usize,
    zero_byte_budget: usize,
}

impl<'a> ByteReader<'a> {
    #[inline]
    pub const fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            pos: 0,
            zero_byte_budget: data.len().saturating_add(64),
        }
    }

    #[inline]
    pub const fn pos(&self) -> usize {
        self.pos
    }

    #[inline]
    pub const fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }

    #[inline]
    pub fn remaining_slice(&self) -> &'a [u8] {
        &self.data[self.pos..]
    }

    #[inline]
    pub fn ensure_remaining(&self, needed: usize) -> Result<(), CodecError> {
        let remaining = self.remaining();
        if needed > remaining {
            Err(CodecError::Truncated { needed, remaining })
        } else {
            Ok(())
        }
    }

    #[inline]
    pub fn read_exact(&mut self, len: usize) -> Result<&'a [u8], CodecError> {
        self.ensure_remaining(len)?;
        let start = self.pos;
        self.pos = start + len;
        Ok(&self.data[start..self.pos])
    }

    #[inline]
    pub fn read_array<const N: usize>(&mut self) -> Result<[u8; N], CodecError> {
        let slice = self.read_exact(N)?;
        let mut out = [0u8; N];
        out.copy_from_slice(slice);
        Ok(out)
    }

    #[inline]
    pub fn expect_magic<const N: usize>(&mut self, magic: &[u8; N]) -> Result<(), CodecError> {
        let got = self.read_exact(N)?;
        if got == magic {
            Ok(())
        } else {
            Err(CodecError::BadMagic)
        }
    }

    #[inline]
    pub fn expect_zero(&mut self, len: usize) -> Result<(), CodecError> {
        let slice = self.read_exact(len)?;
        for &b in slice {
            if b != 0 {
                return Err(CodecError::NonZeroReserved(u64::from(b)));
            }
        }
        Ok(())
    }

    #[inline]
    pub fn read_u8(&mut self) -> Result<u8, CodecError> {
        Ok(self.read_exact(1)?[0])
    }

    #[inline]
    pub fn read_i8(&mut self) -> Result<i8, CodecError> {
        Ok(self.read_u8()? as i8)
    }

    /// Reads a strict boolean (`0 => false`, `1 => true`; any other byte returns `BadBool`).
    #[inline]
    pub fn read_bool(&mut self) -> Result<bool, CodecError> {
        match self.read_u8()? {
            0 => Ok(false),
            1 => Ok(true),
            other => Err(CodecError::BadBool(other)),
        }
    }

    #[inline]
    pub fn read_u16(&mut self) -> Result<u16, CodecError> {
        Ok(u16::from_le_bytes(self.read_array::<2>()?))
    }

    #[inline]
    pub fn read_i16(&mut self) -> Result<i16, CodecError> {
        Ok(i16::from_le_bytes(self.read_array::<2>()?))
    }

    #[inline]
    pub fn read_u32(&mut self) -> Result<u32, CodecError> {
        Ok(u32::from_le_bytes(self.read_array::<4>()?))
    }

    #[inline]
    pub fn read_i32(&mut self) -> Result<i32, CodecError> {
        Ok(i32::from_le_bytes(self.read_array::<4>()?))
    }

    #[inline]
    pub fn read_u64(&mut self) -> Result<u64, CodecError> {
        Ok(u64::from_le_bytes(self.read_array::<8>()?))
    }

    #[inline]
    pub fn read_i64(&mut self) -> Result<i64, CodecError> {
        Ok(i64::from_le_bytes(self.read_array::<8>()?))
    }

    /// Reads a canonical little-endian `f32` (`-0.0` -> `+0.0`, NaN -> canonical quiet NaN).
    #[inline]
    pub fn read_f32(&mut self) -> Result<f32, CodecError> {
        let v = f32::from_le_bytes(self.read_array::<4>()?);
        Ok(f32::from_bits(canonical_f32_bits(v)))
    }

    /// Reads a raw little-endian `f32` preserving exact IEEE-754 bit patterns.
    #[inline]
    pub fn read_f32_raw(&mut self) -> Result<f32, CodecError> {
        Ok(f32::from_le_bytes(self.read_array::<4>()?))
    }

    /// Reads a canonical little-endian `f64` (`-0.0` -> `+0.0`, NaN -> canonical quiet NaN).
    #[inline]
    pub fn read_f64(&mut self) -> Result<f64, CodecError> {
        let v = f64::from_le_bytes(self.read_array::<8>()?);
        Ok(f64::from_bits(canonical_f64_bits(v)))
    }

    /// Reads a raw little-endian `f64` preserving exact IEEE-754 bit patterns.
    #[inline]
    pub fn read_f64_raw(&mut self) -> Result<f64, CodecError> {
        Ok(f64::from_le_bytes(self.read_array::<8>()?))
    }

    /// Reads a `u32` sequence length bounded by `max` AND guaranteed to fit `min_elem_bytes`
    /// within the remaining slice (prevents OOM pre-allocation on malicious buffers).
    #[inline]
    pub fn read_len(&mut self, max: usize, min_elem_bytes: usize) -> Result<usize, CodecError> {
        let len = self.read_u32()? as usize;
        if len > max {
            return Err(CodecError::LenOverflow { len, max });
        }
        if let Some(max_possible) = self.remaining().checked_div(min_elem_bytes)
            && len > max_possible
        {
            return Err(CodecError::Truncated {
                needed: len.saturating_mul(min_elem_bytes),
                remaining: self.remaining(),
            });
        }
        Ok(len)
    }

    /// Reads a `u32`-length-prefixed byte slice bounded by `max`.
    #[inline]
    pub fn read_bytes(&mut self, max: usize) -> Result<&'a [u8], CodecError> {
        let len = self.read_u32()? as usize;
        if len > max {
            return Err(CodecError::LenOverflow { len, max });
        }
        self.read_exact(len)
    }

    /// Reads a `u16`-length-prefixed byte slice bounded by `max`.
    #[inline]
    pub fn read_bytes_u16(&mut self, max: usize) -> Result<&'a [u8], CodecError> {
        let len = usize::from(self.read_u16()?);
        if len > max {
            return Err(CodecError::LenOverflow { len, max });
        }
        self.read_exact(len)
    }

    /// Reads a `u8`-length-prefixed byte slice bounded by `max`.
    #[inline]
    pub fn read_bytes_u8(&mut self, max: usize) -> Result<&'a [u8], CodecError> {
        let len = usize::from(self.read_u8()?);
        if len > max {
            return Err(CodecError::LenOverflow { len, max });
        }
        self.read_exact(len)
    }

    /// Reads a `u32`-length-prefixed UTF-8 string slice bounded by `max`.
    #[inline]
    pub fn read_str(&mut self, max: usize) -> Result<&'a str, CodecError> {
        let bytes = self.read_bytes(max)?;
        core::str::from_utf8(bytes).map_err(|_| CodecError::InvalidUtf8)
    }

    /// Reads a `u16`-length-prefixed UTF-8 string slice bounded by `max`.
    #[inline]
    pub fn read_str_u16(&mut self, max: usize) -> Result<&'a str, CodecError> {
        let bytes = self.read_bytes_u16(max)?;
        core::str::from_utf8(bytes).map_err(|_| CodecError::InvalidUtf8)
    }

    /// Reads a `u8`-length-prefixed UTF-8 string slice bounded by `max`.
    #[inline]
    pub fn read_str_u8(&mut self, max: usize) -> Result<&'a str, CodecError> {
        let bytes = self.read_bytes_u8(max)?;
        core::str::from_utf8(bytes).map_err(|_| CodecError::InvalidUtf8)
    }

    /// Reads a `u32`-length-prefixed owned `String` bounded by `max`.
    #[inline]
    pub fn read_string(&mut self, max: usize) -> Result<String, CodecError> {
        Ok(self.read_str(max)?.to_owned())
    }

    /// Reads a `u32`-length-prefixed vector bounded by `max` elements and `min_elem_bytes` per item.
    ///
    /// When `min_elem_bytes == 0`, no upfront capacity is allocated (so lying lengths fail with 0
    /// heap allocations) and zero-byte-progress items are bounded to `O(input_len)` across the
    /// reader's lifetime to prevent `Vec<Vec<()>>` CPU-amplification DoS attacks.
    pub fn read_vec<T>(
        &mut self,
        max: usize,
        min_elem_bytes: usize,
        mut decode_item: impl FnMut(&mut Self) -> Result<T, CodecError>,
    ) -> Result<Vec<T>, CodecError> {
        let count = self.read_len(max, min_elem_bytes)?;
        if min_elem_bytes > 0 {
            let mut out = Vec::with_capacity(count);
            for _ in 0..count {
                out.push(decode_item(self)?);
            }
            return Ok(out);
        }
        let mut out = Vec::new();
        for i in 0..count {
            let before = self.pos;
            let item = decode_item(self)?;
            if self.pos == before {
                let remaining_items = count - i;
                if remaining_items > self.zero_byte_budget {
                    return Err(CodecError::LenOverflow {
                        len: count,
                        max: self.zero_byte_budget,
                    });
                }
                self.zero_byte_budget -= 1;
            }
            out.push(item);
        }
        Ok(out)
    }

    /// Reads an optional value tagged by a strict boolean byte (`0 = None`, `1 = Some(T)`).
    #[inline]
    pub fn read_opt<T>(
        &mut self,
        decode_some: impl FnOnce(&mut Self) -> Result<T, CodecError>,
    ) -> Result<Option<T>, CodecError> {
        if self.read_bool()? {
            Ok(Some(decode_some(self)?))
        } else {
            Ok(None)
        }
    }

    /// Decodes a value implementing [`NveDecode`].
    #[inline]
    pub fn read_value<T: NveDecode>(&mut self) -> Result<T, CodecError> {
        T::decode(self)
    }

    /// Asserts that no trailing unread bytes remain.
    #[inline]
    pub fn finish(self) -> Result<(), CodecError> {
        let rem = self.remaining();
        if rem != 0 {
            Err(CodecError::TrailingBytes(rem))
        } else {
            Ok(())
        }
    }
}
