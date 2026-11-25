//! Reusable zero-JSON little-endian binary reader/writer primitives with strict bounds checking.

use crate::error::ServiceError;

/// Upper bound on element count in a single decoded sequence to prevent OOM DoS on corrupt buffers.
pub const MAX_SEQ_ITEMS: usize = 65_536;

/// FNV-1a 64-bit offset basis.
pub const FNV1A_64_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
/// FNV-1a 64-bit prime.
pub const FNV1A_64_PRIME: u64 = 0x0000_0100_0000_01b3;

/// Computes a deterministic 64-bit FNV-1a checksum over `bytes` with zero external dependencies.
pub fn fnv1a64(bytes: &[u8]) -> u64 {
    fnv1a64_update(FNV1A_64_OFFSET, bytes)
}

/// Continues an incremental 64-bit FNV-1a checksum from `state`.
pub fn fnv1a64_update(mut state: u64, bytes: &[u8]) -> u64 {
    for &b in bytes {
        state ^= u64::from(b);
        state = state.wrapping_mul(FNV1A_64_PRIME);
    }
    state
}

/// Validates a non-empty URL containing a scheme (`scheme:...`) without control characters.
pub fn validate_url(url: &str) -> Result<(), ServiceError> {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return Err(ServiceError::InvalidArg("URL must not be empty".into()));
    }
    if trimmed.bytes().any(|b| b < 0x20 || b == 0x7F) {
        return Err(ServiceError::InvalidArg(
            "URL must not contain ASCII control characters".into(),
        ));
    }
    let Some((scheme, rest)) = trimmed.split_once(':') else {
        return Err(ServiceError::InvalidArg("URL must include a scheme".into()));
    };
    if scheme.is_empty()
        || rest.is_empty()
        || !scheme.as_bytes()[0].is_ascii_alphabetic()
        || !scheme
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.'))
    {
        return Err(ServiceError::InvalidArg(format!(
            "invalid URL scheme '{scheme}'"
        )));
    }
    Ok(())
}

/// Little-endian binary encoder for host capability requests, responses, and push events.
#[derive(Debug, Default, Clone)]
pub struct BinWriter {
    buf: Vec<u8>,
}

impl BinWriter {
    pub fn new() -> Self {
        Self { buf: Vec::new() }
    }

    pub fn with_capacity(cap: usize) -> Self {
        Self {
            buf: Vec::with_capacity(cap),
        }
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.buf
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.buf
    }

    pub fn write_u8(&mut self, v: u8) {
        self.buf.push(v);
    }

    pub fn write_bool(&mut self, v: bool) {
        self.buf.push(u8::from(v));
    }

    pub fn write_u16(&mut self, v: u16) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    pub fn write_u32(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    pub fn write_i32(&mut self, v: i32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    pub fn write_u64(&mut self, v: u64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    pub fn write_i64(&mut self, v: i64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    pub fn write_f32(&mut self, v: f32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    pub fn write_f64(&mut self, v: f64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    pub fn write_raw(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    pub fn write_bytes(&mut self, bytes: &[u8]) -> Result<(), ServiceError> {
        let len = u32::try_from(bytes.len())
            .map_err(|_| ServiceError::InvalidArg("byte slice exceeds u32::MAX".into()))?;
        self.write_u32(len);
        self.buf.extend_from_slice(bytes);
        Ok(())
    }

    pub fn write_str(&mut self, s: &str) -> Result<(), ServiceError> {
        self.write_bytes(s.as_bytes())
    }

    pub fn write_opt_str(&mut self, opt: Option<&str>) -> Result<(), ServiceError> {
        match opt {
            Some(s) => {
                self.write_u8(1);
                self.write_str(s)
            }
            None => {
                self.write_u8(0);
                Ok(())
            }
        }
    }

    pub fn write_opt_bytes(&mut self, opt: Option<&[u8]>) -> Result<(), ServiceError> {
        match opt {
            Some(b) => {
                self.write_u8(1);
                self.write_bytes(b)
            }
            None => {
                self.write_u8(0);
                Ok(())
            }
        }
    }

    pub fn write_opt_u32(&mut self, opt: Option<u32>) {
        match opt {
            Some(v) => {
                self.write_u8(1);
                self.write_u32(v);
            }
            None => self.write_u8(0),
        }
    }

    pub fn write_opt_u64(&mut self, opt: Option<u64>) {
        match opt {
            Some(v) => {
                self.write_u8(1);
                self.write_u64(v);
            }
            None => self.write_u8(0),
        }
    }

    pub fn write_opt_f32(&mut self, opt: Option<f32>) {
        match opt {
            Some(v) => {
                self.write_u8(1);
                self.write_f32(v);
            }
            None => self.write_u8(0),
        }
    }

    pub fn write_opt_f64(&mut self, opt: Option<f64>) {
        match opt {
            Some(v) => {
                self.write_u8(1);
                self.write_f64(v);
            }
            None => self.write_u8(0),
        }
    }

    pub fn write_str_vec(&mut self, items: &[String]) -> Result<(), ServiceError> {
        let len = u32::try_from(items.len())
            .map_err(|_| ServiceError::InvalidArg("string list exceeds u32::MAX".into()))?;
        self.write_u32(len);
        for item in items {
            self.write_str(item)?;
        }
        Ok(())
    }
}

/// Strictly bounds-checked little-endian binary decoder.
#[derive(Debug, Clone)]
pub struct BinReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> BinReader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    pub fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }

    pub fn read_exact(&mut self, len: usize) -> Result<&'a [u8], ServiceError> {
        let end = self
            .pos
            .checked_add(len)
            .ok_or_else(|| ServiceError::Codec("cursor overflow".into()))?;
        if end > self.data.len() {
            return Err(ServiceError::Codec(format!(
                "unexpected EOF: need {len} bytes, only {} remaining",
                self.remaining()
            )));
        }
        let slice = &self.data[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    pub fn read_u8(&mut self) -> Result<u8, ServiceError> {
        Ok(self.read_exact(1)?[0])
    }

    pub fn read_bool(&mut self) -> Result<bool, ServiceError> {
        match self.read_u8()? {
            0 => Ok(false),
            1 => Ok(true),
            other => Err(ServiceError::Codec(format!("invalid boolean byte {other}"))),
        }
    }

    pub fn read_u16(&mut self) -> Result<u16, ServiceError> {
        let b = self.read_exact(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    pub fn read_u32(&mut self) -> Result<u32, ServiceError> {
        let b = self.read_exact(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub fn read_i32(&mut self) -> Result<i32, ServiceError> {
        let b = self.read_exact(4)?;
        Ok(i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub fn read_u64(&mut self) -> Result<u64, ServiceError> {
        let b = self.read_exact(8)?;
        Ok(u64::from_le_bytes([
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        ]))
    }

    pub fn read_i64(&mut self) -> Result<i64, ServiceError> {
        let b = self.read_exact(8)?;
        Ok(i64::from_le_bytes([
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        ]))
    }

    pub fn read_f32(&mut self) -> Result<f32, ServiceError> {
        let b = self.read_exact(4)?;
        Ok(f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub fn read_f64(&mut self) -> Result<f64, ServiceError> {
        let b = self.read_exact(8)?;
        Ok(f64::from_le_bytes([
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        ]))
    }

    pub fn read_bytes(&mut self) -> Result<&'a [u8], ServiceError> {
        let len = self.read_u32()? as usize;
        self.read_exact(len)
    }

    pub fn read_str(&mut self) -> Result<&'a str, ServiceError> {
        let bytes = self.read_bytes()?;
        std::str::from_utf8(bytes)
            .map_err(|e| ServiceError::Codec(format!("invalid UTF-8 string: {e}")))
    }

    pub fn read_string(&mut self) -> Result<String, ServiceError> {
        Ok(self.read_str()?.to_owned())
    }

    pub fn read_opt_string(&mut self) -> Result<Option<String>, ServiceError> {
        if self.read_bool()? {
            Ok(Some(self.read_string()?))
        } else {
            Ok(None)
        }
    }

    pub fn read_opt_bytes(&mut self) -> Result<Option<Vec<u8>>, ServiceError> {
        if self.read_bool()? {
            Ok(Some(self.read_bytes()?.to_vec()))
        } else {
            Ok(None)
        }
    }

    pub fn read_opt_u32(&mut self) -> Result<Option<u32>, ServiceError> {
        if self.read_bool()? {
            Ok(Some(self.read_u32()?))
        } else {
            Ok(None)
        }
    }

    pub fn read_opt_u64(&mut self) -> Result<Option<u64>, ServiceError> {
        if self.read_bool()? {
            Ok(Some(self.read_u64()?))
        } else {
            Ok(None)
        }
    }

    pub fn read_opt_f32(&mut self) -> Result<Option<f32>, ServiceError> {
        if self.read_bool()? {
            Ok(Some(self.read_f32()?))
        } else {
            Ok(None)
        }
    }

    pub fn read_opt_f64(&mut self) -> Result<Option<f64>, ServiceError> {
        if self.read_bool()? {
            Ok(Some(self.read_f64()?))
        } else {
            Ok(None)
        }
    }

    pub fn read_seq_len(&mut self, min_item_bytes: usize) -> Result<usize, ServiceError> {
        let count = self.read_u32()? as usize;
        if count > MAX_SEQ_ITEMS {
            return Err(ServiceError::Codec(format!(
                "sequence length {count} exceeds limit {MAX_SEQ_ITEMS}"
            )));
        }
        if min_item_bytes > 0 && count > self.remaining() / min_item_bytes {
            return Err(ServiceError::Codec(format!(
                "sequence length {count} exceeds remaining {} bytes",
                self.remaining()
            )));
        }
        Ok(count)
    }

    pub fn read_string_vec(&mut self) -> Result<Vec<String>, ServiceError> {
        let count = self.read_seq_len(4)?;
        let mut out = Vec::with_capacity(count);
        for _ in 0..count {
            out.push(self.read_string()?);
        }
        Ok(out)
    }

    pub fn finish(self) -> Result<(), ServiceError> {
        if self.remaining() != 0 {
            return Err(ServiceError::Codec(format!(
                "unexpected {} trailing bytes",
                self.remaining()
            )));
        }
        Ok(())
    }
}
