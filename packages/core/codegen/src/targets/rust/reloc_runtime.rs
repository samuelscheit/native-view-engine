//! Static Rust 2024 runtime primitives and `SlabBuilder` template emitted by `reloc::emit_rust`.

use crate::writer::CodeWriter;

use crate::schema::reloc::ir::{
    RelocSchema, RELOC_HEADER_ALIGN, RELOC_HEADER_SIZE, RELOC_SLICE_ALIGN, RELOC_SLICE_SIZE,
};

pub(super) fn emit_core_primitives(out: &mut CodeWriter) {
    writeln!(
        out,
        r#"/// Verification tier controlling zero-copy slab validation cost.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VerifyMode {{
    /// Untrusted inputs (network sockets, external files): full bounds, alignment,
    /// magic/schema hash, UTF-8, bool, and enum discriminant verification.
    Full = 0,
    /// Trusted local `mmap` caches / compiled-in static assets: O(1) alignment, magic,
    /// schema hash, and root length verification.
    HeaderOnly = 1,
    /// Zero runtime checks (raw pointer cast). Caller guarantees all invariants.
    Unchecked = 2,
}}

/// Error returned when a binary slab fails verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RelocError {{
    Truncated,
    Misaligned,
    InvalidMagic,
    SchemaHashMismatch,
    OutOfBounds,
    LengthOverflow,
    InvalidUtf8,
    InvalidBool,
    InvalidEnumDiscriminant,
    DepthExceeded,
}}

impl fmt::Display for RelocError {{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {{
        f.write_str(match self {{
            Self::Truncated => "reloc slab truncated",
            Self::Misaligned => "reloc slab or slice misaligned",
            Self::InvalidMagic => "reloc slab magic mismatch",
            Self::SchemaHashMismatch => "reloc slab schema hash mismatch",
            Self::OutOfBounds => "reloc slice or root out of bounds",
            Self::LengthOverflow => "reloc slice byte length overflow",
            Self::InvalidUtf8 => "reloc string contains invalid UTF-8",
            Self::InvalidBool => "reloc bool byte is neither 0 nor 1",
            Self::InvalidEnumDiscriminant => "reloc enum discriminant is invalid",
            Self::DepthExceeded => "reloc verification depth limit exceeded",
        }})
    }}
}}

/// Fixed 32-byte 8-byte-aligned binary slab header.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RelocHeader {{
    pub magic: [u8; 8],
    pub schema_hash: u64,
    pub total_len: u32,
    pub flags: u32,
    pub root_offset: u32,
    pub _pad: u32,
}}

impl RelocHeader {{
    /// Zero-copy view of the header as raw bytes (zero implicit padding).
    #[inline]
    pub fn as_bytes(&self) -> &[u8] {{
        unsafe {{ slice::from_raw_parts((self as *const Self).cast::<u8>(), size_of::<Self>()) }}
    }}
}}

/// ASLR-safe 8-byte relative slice (`offset` relative to slab base + element `len`).
#[repr(C)]
pub struct RelocSlice<T> {{
    pub offset: u32,
    pub len: u32,
    _marker: PhantomData<T>,
}}

impl<T> Copy for RelocSlice<T> {{}}
impl<T> Clone for RelocSlice<T> {{
    #[inline]
    fn clone(&self) -> Self {{
        *self
    }}
}}
impl<T> PartialEq for RelocSlice<T> {{
    #[inline]
    fn eq(&self, other: &Self) -> bool {{
        self.offset == other.offset && self.len == other.len
    }}
}}
impl<T> Eq for RelocSlice<T> {{}}
impl<T> fmt::Debug for RelocSlice<T> {{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {{
        f.debug_struct("RelocSlice")
            .field("offset", &self.offset)
            .field("len", &self.len)
            .finish()
    }}
}}

impl<T> RelocSlice<T> {{
    pub const EMPTY: Self = Self {{
        offset: 0,
        len: 0,
        _marker: PhantomData,
    }};

    #[inline]
    pub const fn new(offset: u32, len: u32) -> Self {{
        Self {{
            offset,
            len,
            _marker: PhantomData,
        }}
    }}

    #[inline]
    pub const fn len(&self) -> usize {{
        self.len as usize
    }}

    #[inline]
    pub const fn is_empty(&self) -> bool {{
        self.len == 0
    }}

    /// Zero-copy slice resolution with 0 runtime checks.
    ///
    /// # Safety
    /// Caller must ensure `slab` was validated (e.g. via `VerifyMode::Full`) and `self` points
    /// to an aligned, in-bounds, valid sequence of `self.len` elements of `T`.
    #[inline]
    pub unsafe fn as_slice_unchecked<'a>(&self, slab: &'a [u8]) -> &'a [T] {{
        if self.len == 0 {{
            return &[];
        }}
        unsafe {{
            let ptr = slab.as_ptr().add(self.offset as usize).cast::<T>();
            slice::from_raw_parts(ptr, self.len as usize)
        }}
    }}
}}

impl<T: RelocVerify> RelocSlice<T> {{
    /// Safely validates bounds, alignment, and element invariants before returning `&'a [T]`.
    pub fn resolve<'a>(&self, slab: &'a [u8]) -> Result<&'a [T], RelocError> {{
        verify_reloc_slice::<T>(slab, slab.len(), self.offset, self.len, 0)?;
        Ok(unsafe {{ self.as_slice_unchecked(slab) }})
    }}
}}

/// ASLR-safe 8-byte relative UTF-8 string slice (`offset` relative to slab base + byte `len`).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RelocStr {{
    pub offset: u32,
    pub len: u32,
}}

impl RelocStr {{
    pub const EMPTY: Self = Self {{ offset: 0, len: 0 }};

    #[inline]
    pub const fn new(offset: u32, len: u32) -> Self {{
        Self {{ offset, len }}
    }}

    #[inline]
    pub const fn len(&self) -> usize {{
        self.len as usize
    }}

    #[inline]
    pub const fn is_empty(&self) -> bool {{
        self.len == 0
    }}

    /// Zero-copy byte view of this descriptor (zero implicit padding).
    #[inline]
    pub fn as_bytes(&self) -> &[u8] {{
        unsafe {{ slice::from_raw_parts((self as *const Self).cast::<u8>(), size_of::<Self>()) }}
    }}

    /// Safely validates bounds and UTF-8 validity before returning `&'a str`.
    pub fn as_str<'a>(&self, slab: &'a [u8]) -> Result<&'a str, RelocError> {{
        verify_reloc_str(slab, slab.len(), self.offset, self.len)?;
        if self.len == 0 {{
            return Ok("");
        }}
        let start = self.offset as usize;
        let end = start + self.len as usize;
        core::str::from_utf8(&slab[start..end]).map_err(|_| RelocError::InvalidUtf8)
    }}

    /// Zero-copy UTF-8 string resolution with 0 runtime checks.
    ///
    /// # Safety
    /// Caller must ensure `slab` was validated (e.g. via `VerifyMode::Full`).
    #[inline]
    pub unsafe fn as_str_unchecked<'a>(&self, slab: &'a [u8]) -> &'a str {{
        if self.len == 0 {{
            return "";
        }}
        unsafe {{
            let ptr = slab.as_ptr().add(self.offset as usize);
            let bytes = slice::from_raw_parts(ptr, self.len as usize);
            core::str::from_utf8_unchecked(bytes)
        }}
    }}
}}

const _: () = {{
    assert!(size_of::<RelocHeader>() == {RELOC_HEADER_SIZE});
    assert!(align_of::<RelocHeader>() == {RELOC_HEADER_ALIGN});
    assert!(offset_of!(RelocHeader, magic) == 0);
    assert!(offset_of!(RelocHeader, schema_hash) == 8);
    assert!(offset_of!(RelocHeader, total_len) == 16);
    assert!(offset_of!(RelocHeader, flags) == 20);
    assert!(offset_of!(RelocHeader, root_offset) == 24);
    assert!(offset_of!(RelocHeader, _pad) == 28);
    assert!(size_of::<RelocSlice<u32>>() == {RELOC_SLICE_SIZE});
    assert!(align_of::<RelocSlice<u32>>() == {RELOC_SLICE_ALIGN});
    assert!(offset_of!(RelocSlice<u32>, offset) == 0);
    assert!(offset_of!(RelocSlice<u32>, len) == 4);
    assert!(size_of::<RelocStr>() == {RELOC_SLICE_SIZE});
    assert!(align_of::<RelocStr>() == {RELOC_SLICE_ALIGN});
    assert!(offset_of!(RelocStr, offset) == 0);
    assert!(offset_of!(RelocStr, len) == 4);
}};

/// Trait implemented for every schema type supporting raw-byte verification before casting.
pub trait RelocVerify: Sized {{
    const NEEDS_VERIFY: bool;
    fn verify_bytes(slab: &[u8], total_len: usize, base: usize, depth: usize) -> Result<(), RelocError>;
    fn write_le_bytes(&self, dst: &mut [u8]);
}}

#[inline]
fn read_u32_le(slab: &[u8], off: usize) -> u32 {{
    u32::from_le_bytes([slab[off], slab[off + 1], slab[off + 2], slab[off + 3]])
}}

#[inline]
fn read_u64_le(slab: &[u8], off: usize) -> u64 {{
    u64::from_le_bytes([
        slab[off],
        slab[off + 1],
        slab[off + 2],
        slab[off + 3],
        slab[off + 4],
        slab[off + 5],
        slab[off + 6],
        slab[off + 7],
    ])
}}

fn verify_reloc_str(slab: &[u8], total_len: usize, offset: u32, len: u32) -> Result<(), RelocError> {{
    let off = offset as usize;
    let byte_len = len as usize;
    if byte_len == 0 {{
        return if off <= total_len {{ Ok(()) }} else {{ Err(RelocError::OutOfBounds) }};
    }}
    if off < HEADER_SIZE {{
        return Err(RelocError::OutOfBounds);
    }}
    offset.checked_add(len).ok_or(RelocError::LengthOverflow)?;
    let end = off.checked_add(byte_len).ok_or(RelocError::LengthOverflow)?;
    if end > total_len {{
        return Err(RelocError::OutOfBounds);
    }}
    core::str::from_utf8(&slab[off..end]).map_err(|_| RelocError::InvalidUtf8)?;
    Ok(())
}}

fn verify_reloc_slice<T: RelocVerify>(
    slab: &[u8],
    total_len: usize,
    offset: u32,
    len: u32,
    depth: usize,
) -> Result<(), RelocError> {{
    if depth >= MAX_VERIFY_DEPTH {{
        return Err(RelocError::DepthExceeded);
    }}
    let off = offset as usize;
    let count = len as usize;
    if count == 0 {{
        return if off <= total_len {{ Ok(()) }} else {{ Err(RelocError::OutOfBounds) }};
    }}
    if off < HEADER_SIZE {{
        return Err(RelocError::OutOfBounds);
    }}
    let elem_size = size_of::<T>();
    let elem_align = align_of::<T>();
    let byte_len = count.checked_mul(elem_size).ok_or(RelocError::LengthOverflow)?;
    let byte_len_u32 = u32::try_from(byte_len).map_err(|_| RelocError::LengthOverflow)?;
    offset.checked_add(byte_len_u32).ok_or(RelocError::LengthOverflow)?;
    let end = off.checked_add(byte_len).ok_or(RelocError::LengthOverflow)?;
    if end > total_len {{
        return Err(RelocError::OutOfBounds);
    }}
    let abs_addr = (slab.as_ptr() as usize).wrapping_add(off);
    if !off.is_multiple_of(elem_align) || !abs_addr.is_multiple_of(elem_align) {{
        return Err(RelocError::Misaligned);
    }}
    if T::NEEDS_VERIFY {{
        for i in 0..count {{
            T::verify_bytes(slab, total_len, off + i * elem_size, depth + 1)?;
        }}
    }}
    Ok(())
}}

impl RelocVerify for bool {{
    const NEEDS_VERIFY: bool = true;
    #[inline]
    fn verify_bytes(slab: &[u8], _total_len: usize, base: usize, _depth: usize) -> Result<(), RelocError> {{
        match slab[base] {{
            0 | 1 => Ok(()),
            _ => Err(RelocError::InvalidBool),
        }}
    }}
    #[inline]
    fn write_le_bytes(&self, dst: &mut [u8]) {{
        dst[0] = u8::from(*self);
    }}
}}

macro_rules! impl_reloc_scalar {{
    ($($ty:ty),* $(,)?) => {{
        $(
            impl RelocVerify for $ty {{
                const NEEDS_VERIFY: bool = false;
                #[inline]
                fn verify_bytes(_slab: &[u8], _total_len: usize, _base: usize, _depth: usize) -> Result<(), RelocError> {{
                    Ok(())
                }}
                #[inline]
                fn write_le_bytes(&self, dst: &mut [u8]) {{
                    dst[..size_of::<$ty>()].copy_from_slice(&self.to_le_bytes());
                }}
            }}
        )*
    }};
}}

impl_reloc_scalar!(u8, i8, u16, i16, u32, i32, f32, u64, i64, f64);

impl<T: RelocVerify, const N: usize> RelocVerify for [T; N] {{
    const NEEDS_VERIFY: bool = T::NEEDS_VERIFY;
    fn verify_bytes(slab: &[u8], total_len: usize, base: usize, depth: usize) -> Result<(), RelocError> {{
        if T::NEEDS_VERIFY {{
            for i in 0..N {{
                T::verify_bytes(slab, total_len, base + i * size_of::<T>(), depth)?;
            }}
        }}
        Ok(())
    }}
    fn write_le_bytes(&self, dst: &mut [u8]) {{
        let stride = size_of::<T>();
        for (i, elem) in self.iter().enumerate() {{
            elem.write_le_bytes(&mut dst[i * stride..(i + 1) * stride]);
        }}
    }}
}}

impl<T: RelocVerify> RelocVerify for RelocSlice<T> {{
    const NEEDS_VERIFY: bool = true;
    #[inline]
    fn verify_bytes(slab: &[u8], total_len: usize, base: usize, depth: usize) -> Result<(), RelocError> {{
        let offset = read_u32_le(slab, base);
        let len = read_u32_le(slab, base + 4);
        verify_reloc_slice::<T>(slab, total_len, offset, len, depth)
    }}
    #[inline]
    fn write_le_bytes(&self, dst: &mut [u8]) {{
        dst[0..4].copy_from_slice(&self.offset.to_le_bytes());
        dst[4..8].copy_from_slice(&self.len.to_le_bytes());
    }}
}}

impl RelocVerify for RelocStr {{
    const NEEDS_VERIFY: bool = true;
    #[inline]
    fn verify_bytes(slab: &[u8], total_len: usize, base: usize, _depth: usize) -> Result<(), RelocError> {{
        let offset = read_u32_le(slab, base);
        let len = read_u32_le(slab, base + 4);
        verify_reloc_str(slab, total_len, offset, len)
    }}
    #[inline]
    fn write_le_bytes(&self, dst: &mut [u8]) {{
        dst[0..4].copy_from_slice(&self.offset.to_le_bytes());
        dst[4..8].copy_from_slice(&self.len.to_le_bytes());
    }}
}}
"#
    );
}

pub(super) fn emit_slab_builder(out: &mut CodeWriter, schema: &RelocSchema, slab_align: usize) {
    let root = &schema.root;
    let word_align = slab_align.max(8);
    writeln!(
        out,
        r#"/// Owned byte buffer guaranteed aligned to `SLAB_ALIGN` ({word_align} bytes).
#[repr(C, align({word_align}))]
#[derive(Clone, Copy)]
struct AlignedBlock([u8; {word_align}]);

#[derive(Clone)]
pub struct AlignedSlab {{
    blocks: Vec<AlignedBlock>,
    len: usize,
}}

impl AlignedSlab {{
    pub fn from_bytes(bytes: &[u8]) -> Self {{
        let n_blocks = bytes.len().div_ceil({word_align}).max(1);
        let mut blocks = vec![AlignedBlock([0u8; {word_align}]); n_blocks];
        let dst = unsafe {{
            slice::from_raw_parts_mut(blocks.as_mut_ptr().cast::<u8>(), n_blocks * {word_align})
        }};
        dst[..bytes.len()].copy_from_slice(bytes);
        Self {{
            blocks,
            len: bytes.len(),
        }}
    }}

    #[inline]
    pub fn as_slice(&self) -> &[u8] {{
        unsafe {{ slice::from_raw_parts(self.blocks.as_ptr().cast::<u8>(), self.len) }}
    }}

    #[inline]
    pub fn as_mut_slice(&mut self) -> &mut [u8] {{
        unsafe {{ slice::from_raw_parts_mut(self.blocks.as_mut_ptr().cast::<u8>(), self.len) }}
    }}
}}

/// Zero-serialization builder producing an aligned binary slab readable by Rust and C++20.
pub struct SlabBuilder {{
    buf: Vec<u8>,
}}

impl Default for SlabBuilder {{
    fn default() -> Self {{
        Self::new()
    }}
}}

impl SlabBuilder {{
    pub fn new() -> Self {{
        Self {{
            buf: vec![0u8; HEADER_SIZE],
        }}
    }}

    fn align_buf(&mut self, align: usize) {{
        if align <= 1 {{
            return;
        }}
        let rem = self.buf.len() % align;
        if rem != 0 {{
            self.buf.resize(self.buf.len() + (align - rem), 0);
        }}
    }}

    pub fn push_str(&mut self, s: &str) -> RelocStr {{
        if s.is_empty() {{
            return RelocStr::EMPTY;
        }}
        let offset = u32::try_from(self.buf.len()).expect("slab offset exceeds u32::MAX");
        let len = u32::try_from(s.len()).expect("string len exceeds u32::MAX");
        self.buf.extend_from_slice(s.as_bytes());
        RelocStr::new(offset, len)
    }}

    pub fn push_slice<T: RelocVerify>(&mut self, items: &[T]) -> RelocSlice<T> {{
        if items.is_empty() {{
            return RelocSlice::EMPTY;
        }}
        self.align_buf(align_of::<T>());
        let offset = u32::try_from(self.buf.len()).expect("slab offset exceeds u32::MAX");
        let len = u32::try_from(items.len()).expect("slice len exceeds u32::MAX");
        let elem_size = size_of::<T>();
        let start = self.buf.len();
        self.buf.resize(start + items.len() * elem_size, 0);
        for (i, item) in items.iter().enumerate() {{
            let off = start + i * elem_size;
            item.write_le_bytes(&mut self.buf[off..off + elem_size]);
        }}
        RelocSlice::new(offset, len)
    }}

    pub fn finish(mut self, root: &{root}, flags: u32) -> AlignedSlab {{
        self.align_buf(align_of::<{root}>());
        let root_offset = u32::try_from(self.buf.len()).expect("root offset exceeds u32::MAX");
        let root_size = size_of::<{root}>();
        let start = self.buf.len();
        self.buf.resize(start + root_size, 0);
        root.write_le_bytes(&mut self.buf[start..start + root_size]);
        self.align_buf(SLAB_ALIGN);
        let total_len = u32::try_from(self.buf.len()).expect("slab total_len exceeds u32::MAX");
        let header = RelocHeader {{
            magic: SCHEMA_MAGIC,
            schema_hash: SCHEMA_HASH,
            total_len,
            flags,
            root_offset,
            _pad: 0,
        }};
        self.buf[0..HEADER_SIZE].copy_from_slice(header.as_bytes());
        AlignedSlab::from_bytes(&self.buf)
    }}
}}
"#
    );
}
