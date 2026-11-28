//! Reusable little-endian binary writers (`ByteWriter`, `SliceWriter`, `ByteSink`, `write_fixed`).

use crate::{canonical_f32_bits, canonical_f64_bits, CodecError, NveEncode};

/// Shared little-endian primitive writing sink implemented by both [`ByteWriter`] and
/// [`SliceWriter`], enabling fixed-layout wire types to write one shared serialization path.
pub trait ByteSink {
    fn put_raw(&mut self, bytes: &[u8]) -> Result<(), CodecError>;
    fn put_zeroes(&mut self, count: usize) -> Result<(), CodecError>;

    #[inline]
    fn put_u8(&mut self, v: u8) -> Result<(), CodecError> {
        self.put_raw(&[v])
    }

    #[inline]
    fn put_i8(&mut self, v: i8) -> Result<(), CodecError> {
        self.put_raw(&[v as u8])
    }

    #[inline]
    fn put_bool(&mut self, v: bool) -> Result<(), CodecError> {
        self.put_raw(&[u8::from(v)])
    }

    #[inline]
    fn put_u16(&mut self, v: u16) -> Result<(), CodecError> {
        self.put_raw(&v.to_le_bytes())
    }

    #[inline]
    fn put_i16(&mut self, v: i16) -> Result<(), CodecError> {
        self.put_raw(&v.to_le_bytes())
    }

    #[inline]
    fn put_u32(&mut self, v: u32) -> Result<(), CodecError> {
        self.put_raw(&v.to_le_bytes())
    }

    #[inline]
    fn put_i32(&mut self, v: i32) -> Result<(), CodecError> {
        self.put_raw(&v.to_le_bytes())
    }

    #[inline]
    fn put_u64(&mut self, v: u64) -> Result<(), CodecError> {
        self.put_raw(&v.to_le_bytes())
    }

    #[inline]
    fn put_i64(&mut self, v: i64) -> Result<(), CodecError> {
        self.put_raw(&v.to_le_bytes())
    }

    #[inline]
    fn put_f32(&mut self, v: f32) -> Result<(), CodecError> {
        self.put_raw(&canonical_f32_bits(v).to_le_bytes())
    }

    #[inline]
    fn put_f32_raw(&mut self, v: f32) -> Result<(), CodecError> {
        self.put_raw(&v.to_bits().to_le_bytes())
    }

    #[inline]
    fn put_f64(&mut self, v: f64) -> Result<(), CodecError> {
        self.put_raw(&canonical_f64_bits(v).to_le_bytes())
    }

    #[inline]
    fn put_f64_raw(&mut self, v: f64) -> Result<(), CodecError> {
        self.put_raw(&v.to_bits().to_le_bytes())
    }

    #[inline]
    fn put_len(&mut self, len: usize) -> Result<(), CodecError> {
        self.put_len_max(len, u32::MAX as usize)
    }

    #[inline]
    fn put_len_max(&mut self, len: usize, max: usize) -> Result<(), CodecError> {
        let limit = max.min(u32::MAX as usize);
        if len > limit {
            return Err(CodecError::LenOverflow { len, max: limit });
        }
        self.put_u32(len as u32)
    }

    #[inline]
    fn put_bytes(&mut self, bytes: &[u8]) -> Result<(), CodecError> {
        self.put_bytes_max(bytes, u32::MAX as usize)
    }

    #[inline]
    fn put_bytes_max(&mut self, bytes: &[u8], max: usize) -> Result<(), CodecError> {
        self.put_len_max(bytes.len(), max)?;
        self.put_raw(bytes)
    }

    #[inline]
    fn put_bytes_u16(&mut self, bytes: &[u8]) -> Result<(), CodecError> {
        let raw = u16::try_from(bytes.len()).map_err(|_| CodecError::LenOverflow {
            len: bytes.len(),
            max: usize::from(u16::MAX),
        })?;
        self.put_u16(raw)?;
        self.put_raw(bytes)
    }

    #[inline]
    fn put_bytes_u8(&mut self, bytes: &[u8]) -> Result<(), CodecError> {
        let raw = u8::try_from(bytes.len()).map_err(|_| CodecError::LenOverflow {
            len: bytes.len(),
            max: usize::from(u8::MAX),
        })?;
        self.put_u8(raw)?;
        self.put_raw(bytes)
    }

    #[inline]
    fn put_str(&mut self, s: &str) -> Result<(), CodecError> {
        self.put_bytes(s.as_bytes())
    }

    #[inline]
    fn put_str_max(&mut self, s: &str, max: usize) -> Result<(), CodecError> {
        self.put_bytes_max(s.as_bytes(), max)
    }

    #[inline]
    fn put_str_u16(&mut self, s: &str) -> Result<(), CodecError> {
        self.put_bytes_u16(s.as_bytes())
    }

    #[inline]
    fn put_str_u8(&mut self, s: &str) -> Result<(), CodecError> {
        self.put_bytes_u8(s.as_bytes())
    }

    #[inline]
    fn put_opt<T>(
        &mut self,
        opt: Option<T>,
        write_some: impl FnOnce(&mut Self, T) -> Result<(), CodecError>,
    ) -> Result<(), CodecError>
    where
        Self: Sized,
    {
        match opt {
            Some(v) => {
                self.put_bool(true)?;
                write_some(self, v)
            }
            None => self.put_bool(false),
        }
    }
}

/// Fallible zero-allocation helper for fixed-size wire layouts (`N` bytes exact).
#[inline]
pub fn try_write_fixed<const N: usize>(
    write: impl FnOnce(&mut SliceWriter<'_>) -> Result<(), CodecError>,
) -> Result<[u8; N], CodecError> {
    let mut out = [0u8; N];
    let mut w = SliceWriter::new(&mut out);
    write(&mut w)?;
    w.finish()?;
    Ok(out)
}

/// Zero-allocation helper for infallible fixed-size wire layouts (`N` bytes exact).
///
/// Verifies in both debug and release builds that `write` succeeds and fills all `N` bytes with
/// zero unused trailing space.
#[inline]
pub fn write_fixed<const N: usize>(
    write: impl FnOnce(&mut SliceWriter<'_>) -> Result<(), CodecError>,
) -> [u8; N] {
    let mut out = [0u8; N];
    write_fixed_into(&mut out, write);
    out
}

/// Zero-allocation helper filling a caller-owned `[u8; N]` fixed-size wire buffer.
#[inline]
pub fn write_fixed_into<const N: usize>(
    dst: &mut [u8; N],
    write: impl FnOnce(&mut SliceWriter<'_>) -> Result<(), CodecError>,
) {
    let mut w = SliceWriter::new(dst);
    write(&mut w).expect("fixed-size codec layout write must fit within N bytes");
    w.finish()
        .expect("fixed-size codec layout must fill all N bytes");
}

/// Reusable little-endian binary writer.
#[derive(Debug, Default, Clone)]
pub struct ByteWriter {
    buf: Vec<u8>,
}

impl ByteSink for ByteWriter {
    #[inline]
    fn put_raw(&mut self, bytes: &[u8]) -> Result<(), CodecError> {
        self.write_raw(bytes);
        Ok(())
    }

    #[inline]
    fn put_zeroes(&mut self, count: usize) -> Result<(), CodecError> {
        self.write_zeroes(count)
    }
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
    pub fn write_zeroes(&mut self, count: usize) -> Result<(), CodecError> {
        let max = (isize::MAX as usize).saturating_sub(self.buf.len());
        if count > max || self.buf.try_reserve(count).is_err() {
            return Err(CodecError::LenOverflow { len: count, max });
        }
        self.buf.resize(self.buf.len() + count, 0);
        Ok(())
    }

    #[inline]
    pub fn patch_u32(&mut self, offset: usize, v: u32) -> Result<(), CodecError> {
        let end = offset.checked_add(4).ok_or(CodecError::LenOverflow {
            len: offset,
            max: self.buf.len().saturating_sub(4),
        })?;
        let Some(dst) = self.buf.get_mut(offset..end) else {
            return Err(CodecError::Truncated {
                needed: 4,
                remaining: self.buf.len().saturating_sub(offset),
            });
        };
        dst.copy_from_slice(&v.to_le_bytes());
        Ok(())
    }

    #[inline]
    pub fn write_len(&mut self, len: usize) -> Result<(), CodecError> {
        self.write_len_max(len, u32::MAX as usize)
    }

    /// Writes a `u32` length prefix bounded by `max` (and `u32::MAX`), rejecting oversized
    /// payloads at encode time so an encoder never emits bytes its own decoder rejects.
    #[inline]
    pub fn write_len_max(&mut self, len: usize, max: usize) -> Result<(), CodecError> {
        let limit = max.min(u32::MAX as usize);
        if len > limit {
            return Err(CodecError::LenOverflow { len, max: limit });
        }
        self.write_u32(len as u32);
        Ok(())
    }

    #[inline]
    pub fn write_bytes(&mut self, bytes: &[u8]) -> Result<(), CodecError> {
        self.write_bytes_max(bytes, u32::MAX as usize)
    }

    #[inline]
    pub fn write_bytes_max(&mut self, bytes: &[u8], max: usize) -> Result<(), CodecError> {
        self.write_len_max(bytes.len(), max)?;
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
    pub fn write_str_max(&mut self, s: &str, max: usize) -> Result<(), CodecError> {
        self.write_bytes_max(s.as_bytes(), max)
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

/// Zero-allocation little-endian binary writer over a caller-owned `&mut [u8]` slice.
#[derive(Debug)]
pub struct SliceWriter<'a> {
    buf: &'a mut [u8],
    pos: usize,
}

impl ByteSink for SliceWriter<'_> {
    #[inline]
    fn put_raw(&mut self, bytes: &[u8]) -> Result<(), CodecError> {
        let remaining = self.remaining();
        if bytes.len() > remaining {
            return Err(CodecError::Truncated {
                needed: bytes.len(),
                remaining,
            });
        }
        let end = self.pos + bytes.len();
        self.buf[self.pos..end].copy_from_slice(bytes);
        self.pos = end;
        Ok(())
    }

    #[inline]
    fn put_zeroes(&mut self, count: usize) -> Result<(), CodecError> {
        let remaining = self.remaining();
        if count > remaining {
            return Err(CodecError::Truncated {
                needed: count,
                remaining,
            });
        }
        let end = self.pos + count;
        self.buf[self.pos..end].fill(0);
        self.pos = end;
        Ok(())
    }
}

macro_rules! delegate_slice_sink {
    () => {
        #[inline]
        pub fn write_raw(&mut self, bytes: &[u8]) -> Result<(), $crate::CodecError> {
            $crate::ByteSink::put_raw(self, bytes)
        }

        #[inline]
        pub fn write_zeroes(&mut self, count: usize) -> Result<(), $crate::CodecError> {
            $crate::ByteSink::put_zeroes(self, count)
        }

        $crate::writer::delegate_slice_sink!(
            write_u8 => put_u8 : u8,
            write_i8 => put_i8 : i8,
            write_bool => put_bool : bool,
            write_u16 => put_u16 : u16,
            write_i16 => put_i16 : i16,
            write_u32 => put_u32 : u32,
            write_i32 => put_i32 : i32,
            write_u64 => put_u64 : u64,
            write_i64 => put_i64 : i64,
            write_f32 => put_f32 : f32,
            write_f32_raw => put_f32_raw : f32,
            write_f64 => put_f64 : f64,
            write_f64_raw => put_f64_raw : f64,
            write_len => put_len : usize,
            write_bytes => put_bytes : &[u8],
            write_bytes_u16 => put_bytes_u16 : &[u8],
            write_bytes_u8 => put_bytes_u8 : &[u8],
            write_str => put_str : &str,
            write_str_u16 => put_str_u16 : &str,
            write_str_u8 => put_str_u8 : &str,
        );

        #[inline]
        pub fn write_len_max(&mut self, len: usize, max: usize) -> Result<(), $crate::CodecError> {
            $crate::ByteSink::put_len_max(self, len, max)
        }

        #[inline]
        pub fn write_bytes_max(
            &mut self,
            bytes: &[u8],
            max: usize,
        ) -> Result<(), $crate::CodecError> {
            $crate::ByteSink::put_bytes_max(self, bytes, max)
        }

        #[inline]
        pub fn write_str_max(&mut self, s: &str, max: usize) -> Result<(), $crate::CodecError> {
            $crate::ByteSink::put_str_max(self, s, max)
        }

        #[inline]
        pub fn write_opt<T>(
            &mut self,
            opt: Option<T>,
            write_some: impl FnOnce(&mut Self, T) -> Result<(), $crate::CodecError>,
        ) -> Result<(), $crate::CodecError> {
            $crate::ByteSink::put_opt(self, opt, write_some)
        }
    };
    ($($write:ident => $put:ident : $ty:ty),+ $(,)?) => {
        $(
            #[inline]
            pub fn $write(&mut self, v: $ty) -> Result<(), $crate::CodecError> {
                $crate::ByteSink::$put(self, v)
            }
        )+
    };
}

pub(crate) use delegate_slice_sink;

impl<'a> SliceWriter<'a> {
    #[inline]
    pub fn new(buf: &'a mut [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    #[inline]
    pub const fn pos(&self) -> usize {
        self.pos
    }

    #[inline]
    pub const fn remaining(&self) -> usize {
        self.buf.len() - self.pos
    }

    #[inline]
    pub fn written(&self) -> &[u8] {
        &self.buf[..self.pos]
    }

    delegate_slice_sink!();

    /// Verifies that the destination slice was filled completely with zero unused trailing bytes.
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
