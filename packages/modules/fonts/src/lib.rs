//! Custom font loader & registry capability (`SERVICE_FONTS = 0x0021`) with direct
//! zero-serialization in-process dispatch ([`FontsBackend`]), TTF/OTF/WOFF/WOFF2/TTC magic
//! validation, deduplication, [`MemoryFonts`], and [`NativeFonts`].

pub mod handler;

use std::collections::BTreeMap;

use nve_host_services::{
    fnv1a64, resolve_typed, BinReader, BinWriter, HostServiceProvider, ServiceError,
};

pub use handler::{dispatch_fonts_wire, FontsBackend, MemoryFonts, NativeFonts};

/// Built-in service ID for custom font loading and registry management.
pub const SERVICE_FONTS: u32 = 0x0021;

pub const METHOD_REGISTER_BYTES: u32 = 1;
pub const METHOD_REGISTER_FILE: u32 = 2;
pub const METHOD_UNREGISTER: u32 = 3;
pub const METHOD_LIST_REGISTERED: u32 = 4;

/// Maximum single font payload size accepted over the host capability boundary (64 MiB).
pub const MAX_FONT_BYTES: usize = 64 * 1024 * 1024;

/// Opaque handle identifying a registered custom font face.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FontHandle(pub u64);

/// Recognized binary font container format validated from header magic bytes.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FontFormat {
    TrueType = 1,
    OpenType = 2,
    Woff = 3,
    Woff2 = 4,
    TrueTypeCollection = 5,
}

impl FontFormat {
    pub const fn from_u8(raw: u8) -> Option<Self> {
        match raw {
            1 => Some(Self::TrueType),
            2 => Some(Self::OpenType),
            3 => Some(Self::Woff),
            4 => Some(Self::Woff2),
            5 => Some(Self::TrueTypeCollection),
            _ => None,
        }
    }
}

/// Metadata describing one registered font face in the host registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisteredFont {
    pub handle: FontHandle,
    pub family: String,
    pub weight: u16,
    pub italic: bool,
    pub format: FontFormat,
    pub byte_len: u64,
    pub content_hash: u64,
    pub source_path: Option<String>,
}

/// Validates font family and OpenType/CSS weight (`1..=1000`).
pub fn validate_font_descriptor(family: &str, weight: u16) -> Result<(), ServiceError> {
    if family.trim().is_empty() {
        return Err(ServiceError::InvalidArg(
            "font family must not be empty".into(),
        ));
    }
    if !(1..=1000).contains(&weight) {
        return Err(ServiceError::InvalidArg(format!(
            "font weight {weight} must be in 1..=1000"
        )));
    }
    Ok(())
}

/// Zero-dependency binary magic & minimum-header validation for TTF, OTF, WOFF, WOFF2, and TTC.
pub fn validate_font_bytes(bytes: &[u8]) -> Result<FontFormat, ServiceError> {
    if bytes.is_empty() {
        return Err(ServiceError::InvalidArg(
            "font payload must not be empty".into(),
        ));
    }
    if bytes.len() > MAX_FONT_BYTES {
        return Err(ServiceError::InvalidArg(format!(
            "font payload exceeds maximum size of {MAX_FONT_BYTES} bytes"
        )));
    }
    if bytes.len() < 12 {
        return Err(ServiceError::InvalidArg(format!(
            "font payload truncated: got {} bytes, need at least 12",
            bytes.len()
        )));
    }
    let magic = &bytes[0..4];
    match magic {
        [0x00, 0x01, 0x00, 0x00] | b"true" => Ok(FontFormat::TrueType),
        b"OTTO" => Ok(FontFormat::OpenType),
        b"ttcf" => Ok(FontFormat::TrueTypeCollection),
        b"wOFF" => {
            if bytes.len() < 44 {
                return Err(ServiceError::InvalidArg(
                    "WOFF payload truncated (header requires at least 44 bytes)".into(),
                ));
            }
            Ok(FontFormat::Woff)
        }
        b"wOF2" => {
            if bytes.len() < 48 {
                return Err(ServiceError::InvalidArg(
                    "WOFF2 payload truncated (header requires at least 48 bytes)".into(),
                ));
            }
            Ok(FontFormat::Woff2)
        }
        _ => Err(ServiceError::InvalidArg(format!(
            "unrecognized font magic {:02X?}",
            magic
        ))),
    }
}

pub fn encode_register_bytes_request(
    family: &str,
    weight: u16,
    italic: bool,
    bytes: &[u8],
) -> Result<Vec<u8>, ServiceError> {
    validate_font_descriptor(family, weight)?;
    validate_font_bytes(bytes)?;
    let mut w = BinWriter::with_capacity(11 + family.len() + bytes.len());
    w.write_str(family)?;
    w.write_u16(weight);
    w.write_bool(italic);
    w.write_bytes(bytes)?;
    Ok(w.into_bytes())
}

pub fn decode_register_bytes_request(
    bytes: &[u8],
) -> Result<(String, u16, bool, Vec<u8>), ServiceError> {
    let mut r = BinReader::new(bytes);
    let family = r.read_string()?;
    let weight = r.read_u16()?;
    let italic = r.read_bool()?;
    let font_bytes = r.read_bytes()?.to_vec();
    r.finish()?;
    validate_font_descriptor(&family, weight)?;
    validate_font_bytes(&font_bytes)?;
    Ok((family, weight, italic, font_bytes))
}

pub fn encode_register_file_request(
    family: &str,
    weight: u16,
    italic: bool,
    path: &str,
) -> Result<Vec<u8>, ServiceError> {
    validate_font_descriptor(family, weight)?;
    if path.trim().is_empty() {
        return Err(ServiceError::InvalidArg(
            "font file path must not be empty".into(),
        ));
    }
    let mut w = BinWriter::with_capacity(11 + family.len() + path.len());
    w.write_str(family)?;
    w.write_u16(weight);
    w.write_bool(italic);
    w.write_str(path)?;
    Ok(w.into_bytes())
}

pub fn decode_register_file_request(
    bytes: &[u8],
) -> Result<(String, u16, bool, String), ServiceError> {
    let mut r = BinReader::new(bytes);
    let family = r.read_string()?;
    let weight = r.read_u16()?;
    let italic = r.read_bool()?;
    let path = r.read_string()?;
    r.finish()?;
    validate_font_descriptor(&family, weight)?;
    if path.trim().is_empty() {
        return Err(ServiceError::InvalidArg(
            "font file path must not be empty".into(),
        ));
    }
    Ok((family, weight, italic, path))
}

pub fn encode_handle(handle: FontHandle) -> Vec<u8> {
    let mut w = BinWriter::with_capacity(8);
    w.write_u64(handle.0);
    w.into_bytes()
}

pub fn decode_handle(bytes: &[u8]) -> Result<FontHandle, ServiceError> {
    let mut r = BinReader::new(bytes);
    let id = r.read_u64()?;
    r.finish()?;
    if id == 0 {
        return Err(ServiceError::InvalidArg(
            "font handle must be non-zero".into(),
        ));
    }
    Ok(FontHandle(id))
}

pub fn encode_unregister_response(removed: bool) -> Vec<u8> {
    vec![u8::from(removed)]
}

pub fn decode_unregister_response(bytes: &[u8]) -> Result<bool, ServiceError> {
    let mut r = BinReader::new(bytes);
    let removed = r.read_bool()?;
    r.finish()?;
    Ok(removed)
}

pub fn encode_list_registered_response(fonts: &[RegisteredFont]) -> Result<Vec<u8>, ServiceError> {
    let count = u32::try_from(fonts.len())
        .map_err(|_| ServiceError::InvalidArg("font list exceeds u32::MAX".into()))?;
    let mut w = BinWriter::with_capacity(4 + fonts.len() * 48);
    w.write_u32(count);
    for font in fonts {
        w.write_u64(font.handle.0);
        w.write_str(&font.family)?;
        w.write_u16(font.weight);
        w.write_bool(font.italic);
        w.write_u8(font.format as u8);
        w.write_u64(font.byte_len);
        w.write_u64(font.content_hash);
        w.write_opt_str(font.source_path.as_deref())?;
    }
    Ok(w.into_bytes())
}

pub fn decode_list_registered_response(bytes: &[u8]) -> Result<Vec<RegisteredFont>, ServiceError> {
    let mut r = BinReader::new(bytes);
    let count = r.read_seq_len(33)?;
    let mut fonts = Vec::with_capacity(count);
    for _ in 0..count {
        let handle = FontHandle(r.read_u64()?);
        let family = r.read_string()?;
        let weight = r.read_u16()?;
        let italic = r.read_bool()?;
        let fmt_raw = r.read_u8()?;
        let format = FontFormat::from_u8(fmt_raw)
            .ok_or_else(|| ServiceError::Codec(format!("invalid FontFormat {fmt_raw}")))?;
        let byte_len = r.read_u64()?;
        let content_hash = r.read_u64()?;
        let source_path = r.read_opt_string()?;
        fonts.push(RegisteredFont {
            handle,
            family,
            weight,
            italic,
            format,
            byte_len,
            content_hash,
            source_path,
        });
    }
    r.finish()?;
    Ok(fonts)
}

type DedupKey = (String, u16, bool, u64, u64);

/// Reusable in-process font registry with magic validation and content-hash deduplication.
#[derive(Debug, Default)]
pub struct FontRegistry {
    next_id: u64,
    by_handle: BTreeMap<FontHandle, RegisteredFont>,
    dedup: BTreeMap<DedupKey, FontHandle>,
}

impl FontRegistry {
    pub fn new() -> Self {
        Self {
            next_id: 1,
            by_handle: BTreeMap::new(),
            dedup: BTreeMap::new(),
        }
    }

    pub fn register_bytes(
        &mut self,
        family: String,
        weight: u16,
        italic: bool,
        bytes: &[u8],
        source_path: Option<String>,
    ) -> Result<FontHandle, ServiceError> {
        validate_font_descriptor(&family, weight)?;
        let format = validate_font_bytes(bytes)?;
        let byte_len = bytes.len() as u64;
        let content_hash = fnv1a64(bytes);
        let key = (family.clone(), weight, italic, content_hash, byte_len);
        if let Some(&existing) = self.dedup.get(&key) {
            return Ok(existing);
        }
        let handle = FontHandle(self.next_id);
        self.next_id = self.next_id.saturating_add(1);
        let entry = RegisteredFont {
            handle,
            family,
            weight,
            italic,
            format,
            byte_len,
            content_hash,
            source_path,
        };
        self.dedup.insert(key, handle);
        self.by_handle.insert(handle, entry);
        Ok(handle)
    }

    pub fn unregister(&mut self, handle: FontHandle) -> bool {
        let Some(removed) = self.by_handle.remove(&handle) else {
            return false;
        };
        let key = (
            removed.family,
            removed.weight,
            removed.italic,
            removed.content_hash,
            removed.byte_len,
        );
        self.dedup.remove(&key);
        true
    }

    pub fn list(&self) -> Vec<RegisteredFont> {
        self.by_handle.values().cloned().collect()
    }
}

/// Typed ergonomic client for custom font loading and registration.
#[derive(Debug, Clone, Copy)]
pub struct FontClient<'a, P: HostServiceProvider + ?Sized> {
    provider: &'a P,
}

impl<'a, P: HostServiceProvider + ?Sized> FontClient<'a, P> {
    pub const fn new(provider: &'a P) -> Self {
        Self { provider }
    }

    pub fn register_bytes(
        &self,
        family: &str,
        weight: u16,
        italic: bool,
        bytes: &[u8],
    ) -> Result<FontHandle, ServiceError> {
        validate_font_descriptor(family, weight)?;
        validate_font_bytes(bytes)?;
        if let Some(backend) = resolve_typed::<dyn FontsBackend>(
            self.provider,
            SERVICE_FONTS,
            METHOD_REGISTER_BYTES,
        ) {
            return backend.register_bytes(family, weight, italic, bytes);
        }
        let req = encode_register_bytes_request(family, weight, italic, bytes)?;
        let resp = self
            .provider
            .invoke_sync(SERVICE_FONTS, METHOD_REGISTER_BYTES, &req)?;
        decode_handle(&resp)
    }

    pub fn register_file(
        &self,
        family: &str,
        weight: u16,
        italic: bool,
        path: &str,
    ) -> Result<FontHandle, ServiceError> {
        validate_font_descriptor(family, weight)?;
        if path.trim().is_empty() {
            return Err(ServiceError::InvalidArg(
                "font file path must not be empty".into(),
            ));
        }
        if let Some(backend) = resolve_typed::<dyn FontsBackend>(
            self.provider,
            SERVICE_FONTS,
            METHOD_REGISTER_FILE,
        ) {
            return backend.register_file(family, weight, italic, path);
        }
        let req = encode_register_file_request(family, weight, italic, path)?;
        let resp = self
            .provider
            .invoke_sync(SERVICE_FONTS, METHOD_REGISTER_FILE, &req)?;
        decode_handle(&resp)
    }

    pub fn unregister(&self, handle: FontHandle) -> Result<bool, ServiceError> {
        if handle.0 == 0 {
            return Err(ServiceError::InvalidArg(
                "font handle must be non-zero".into(),
            ));
        }
        if let Some(backend) =
            resolve_typed::<dyn FontsBackend>(self.provider, SERVICE_FONTS, METHOD_UNREGISTER)
        {
            return backend.unregister(handle);
        }
        let req = encode_handle(handle);
        let resp = self
            .provider
            .invoke_sync(SERVICE_FONTS, METHOD_UNREGISTER, &req)?;
        decode_unregister_response(&resp)
    }

    pub fn list_registered(&self) -> Result<Vec<RegisteredFont>, ServiceError> {
        if let Some(backend) = resolve_typed::<dyn FontsBackend>(
            self.provider,
            SERVICE_FONTS,
            METHOD_LIST_REGISTERED,
        ) {
            return backend.list_registered();
        }
        let resp = self
            .provider
            .invoke_sync(SERVICE_FONTS, METHOD_LIST_REGISTERED, &[])?;
        decode_list_registered_response(&resp)
    }
}
