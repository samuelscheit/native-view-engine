//! Reusable little-endian binary writer.

use crate::{canonical_f32_bits, canonical_f64_bits, CodecError, NveEncode};

/// Reusable little-endian binary writer.
#[derive(Debug, Default, Clone)]
pub struct ByteWriter {
    buf: Vec<u8>,
}

impl ByteWriter {
    #[inline]
    pub const fn new() -> Self {
        Self { buf: Vec::new() }
    }

    #[inline]
    pub fn with_capacity(cap: usize) -> Self {
        Self {
            buf: Vec::with_capacity(cap),
        }
    }

    #[inline]
    pub fn from_vec(buf: Vec<u8>) -> Self {
        Self { buf }
    }

    /// Borrows an existing `Vec<u8>` into a [`ByteWriter`] with zero allocation, restoring it on return.
    #[inline]
    pub fn with_buf<R>(buf: &mut Vec<u8>, f: impl FnOnce(&mut Self) -> R) -> R {
        let mut w = Self {
            buf: core::mem::take(buf),
        };
        let res = f(&mut w);
        *buf = w.buf;
        res
    }

    #[inline]
    pub fn clear(&mut self) {
        self.buf.clear();
    }

    #[inline]
    pub fn reserve(&mut self, additional: usize) {
        self.buf.reserve(additional);
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.buf.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    #[inline]
    pub fn as_slice(&self) -> &[u8] {
        &self.buf
    }

    #[inline]
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        &mut self.buf
    }

    #[inline]
    pub fn into_bytes(self) -> Vec<u8> {
        self.buf
    }

    #[inline]
    pub fn write_u8(&mut self, v: u8) {
        self.buf.push(v);
    }

    #[inline]
    pub fn write_i8(&mut self, v: i8) {
        self.buf.push(v as u8);
    }

    #[inline]
    pub fn write_bool(&mut self, v: bool) {
        self.buf.push(u8::from(v));
    }

    #[inline]
    pub fn write_u16(&mut self, v: u16) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    #[inline]
    pub fn write_i16(&mut self, v: i16) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    #[inline]
    pub fn write_u32(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    #[inline]
    pub fn write_i32(&mut self, v: i32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    #[inline]
    pub fn write_u64(&mut self, v: u64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    #[inline]
    pub fn write_i64(&mut self, v: i64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    /// Writes a canonical little-endian `f32` (`-0.0` -> `+0.0`, NaN -> canonical quiet NaN).
    #[inline]
    pub fn write_f32(&mut self, v: f32) {
        self.buf
            .extend_from_slice(&canonical_f32_bits(v).to_le_bytes());
    }

    /// Writes a raw little-endian `f32` preserving exact IEEE-754 bit patterns.
    #[inline]
    pub fn write_f32_raw(&mut self, v: f32) {
        self.buf.extend_from_slice(&v.to_bits().to_le_bytes());
    }

    /// Writes a canonical little-endian `f64` (`-0.0` -> `+0.0`, NaN -> canonical quiet NaN).
    #[inline]
    pub fn write_f64(&mut self, v: f64) {
        self.buf
            .extend_from_slice(&canonical_f64_bits(v).to_le_bytes());
    }

    /// Writes a raw little-endian `f64` preserving exact IEEE-754 bit patterns.
    #[inline]
    pub fn write_f64_raw(&mut self, v: f64) {
        self.buf.extend_from_slice(&v.to_bits().to_le_bytes());
    }

    #[inline]
    pub fn write_raw(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    #[inline]
    pub fn write_zeroes(&mut self, count: usize) {
        self.buf.resize(self.buf.len() + count, 0);
    }

    #[inline]
    pub fn patch_u32(&mut self, offset: usize, v: u32) {
        if let Some(dst) = self.buf.get_mut(offset..offset + 4) {
            dst.copy_from_slice(&v.to_le_bytes());
        }
    }

    #[inline]
    pub fn write_len(&mut self, len: usize) -> Result<(), CodecError> {
        let raw = u32::try_from(len).map_err(|_| CodecError::LenOverflow {
            len,
            max: u32::MAX as usize,
        })?;
        self.write_u32(raw);
        Ok(())
    }

    #[inline]
    pub fn write_bytes(&mut self, bytes: &[u8]) -> Result<(), CodecError> {
        self.write_len(bytes.len())?;
        self.buf.extend_from_slice(bytes);
        Ok(())
    }

    #[inline]
    pub fn write_bytes_u16(&mut self, bytes: &[u8]) -> Result<(), CodecError> {
        let raw = u16::try_from(bytes.len()).map_err(|_| CodecError::LenOverflow {
            len: bytes.len(),
            max: usize::from(u16::MAX),
        })?;
        self.write_u16(raw);
        self.buf.extend_from_slice(bytes);
        Ok(())
    }

    #[inline]
    pub fn write_bytes_u8(&mut self, bytes: &[u8]) -> Result<(), CodecError> {
        let raw = u8::try_from(bytes.len()).map_err(|_| CodecError::LenOverflow {
            len: bytes.len(),
            max: usize::from(u8::MAX),
        })?;
        self.write_u8(raw);
        self.buf.extend_from_slice(bytes);
        Ok(())
    }

    #[inline]
    pub fn write_str(&mut self, s: &str) -> Result<(), CodecError> {
        self.write_bytes(s.as_bytes())
    }

    #[inline]
    pub fn write_str_u16(&mut self, s: &str) -> Result<(), CodecError> {
        self.write_bytes_u16(s.as_bytes())
    }

    #[inline]
    pub fn write_str_u8(&mut self, s: &str) -> Result<(), CodecError> {
        self.write_bytes_u8(s.as_bytes())
    }

    #[inline]
    pub fn write_opt<T>(
        &mut self,
        opt: Option<T>,
        write_some: impl FnOnce(&mut Self, T) -> Result<(), CodecError>,
    ) -> Result<(), CodecError> {
        match opt {
            Some(v) => {
                self.write_bool(true);
                write_some(self, v)
            }
            None => {
                self.write_bool(false);
                Ok(())
            }
        }
    }

    #[inline]
    pub fn write_value<T: NveEncode>(&mut self, value: &T) -> Result<(), CodecError> {
        value.encode(self)
    }
}
