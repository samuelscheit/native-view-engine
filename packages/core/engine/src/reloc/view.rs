//! Zero-copy `NVERELOC` readers ([`RelocView<'a>`], [`MmapReloc`], [`RelocBundleView<'a>`]),
//! [`encode_reloc_bundle`], and crash-atomic [`write_reloc_atomic`].

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::mem::{align_of, size_of};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::mmap_arena::ReadOnlyMmap;

use super::format::{
    compute_reloc_checksum, RelocBuf, RelocBundleRoot, RelocHeader, RelocPtr, RelocSlice, RelocStr,
    RelocWriter, RELOC_HEADER_SIZE, RELOC_KIND_BUNDLE, RELOC_MAGIC, RELOC_VERSION,
};
use super::{RelocError, RelocPod, VerifyMode};

static ATOMIC_TMP_SEQ: AtomicU64 = AtomicU64::new(0);

#[cfg(any(unix, windows))]
#[inline]
fn current_pid() -> u32 {
    std::process::id()
}

#[cfg(not(any(unix, windows)))]
#[inline]
fn current_pid() -> u32 {
    0
}

/// Zero-copy borrowed view directly over a validated `NVERELOC` byte buffer.
#[derive(Clone, Copy)]
pub struct RelocView<'a> {
    bytes: &'a [u8],
    header: &'a RelocHeader,
    mode: VerifyMode,
}

impl<'a> RelocView<'a> {
    /// Opens a zero-copy view using [`VerifyMode::Full`] or [`VerifyMode::HeaderOnly`].
    pub fn from_bytes(bytes: &'a [u8], mode: VerifyMode) -> Result<Self, RelocError> {
        let effective = match mode {
            VerifyMode::Unchecked => VerifyMode::HeaderOnly,
            other => other,
        };
        let view = Self::from_bytes_header_only(bytes, effective)?;
        if effective == VerifyMode::Full {
            view.verify_checksum()?;
        }
        Ok(view)
    }

    fn from_bytes_header_only(bytes: &'a [u8], mode: VerifyMode) -> Result<Self, RelocError> {
        if bytes.len() < RELOC_HEADER_SIZE {
            return Err(RelocError::Truncated);
        }
        let base_addr = bytes.as_ptr() as usize;
        if !base_addr.is_multiple_of(align_of::<RelocHeader>()) {
            return Err(RelocError::Misaligned {
                expected: align_of::<RelocHeader>(),
                actual: base_addr,
            });
        }
        // SAFETY: `bytes.len() >= 64` and `base_addr` is aligned to `align_of::<RelocHeader>()`.
        let header = unsafe { &*bytes.as_ptr().cast::<RelocHeader>() };
        if header.magic != RELOC_MAGIC {
            return Err(RelocError::InvalidMagic);
        }
        if header.version != RELOC_VERSION {
            return Err(RelocError::UnsupportedVersion(header.version));
        }
        if header.reserved != [0, 0, 0] {
            return Err(RelocError::InvalidReservedHeader);
        }
        let actual_payload = bytes.len() - RELOC_HEADER_SIZE;
        if header.payload_len as usize != actual_payload {
            return Err(RelocError::PayloadLengthMismatch {
                header: header.payload_len,
                actual: actual_payload,
            });
        }
        if header.root_offset != 0 {
            let root = header.root_offset as usize;
            if root < RELOC_HEADER_SIZE || root >= bytes.len() {
                return Err(RelocError::OutOfBounds("root_offset"));
            }
        }
        Ok(Self {
            bytes,
            header,
            mode,
        })
    }

    /// Zero-check direct pointer cast onto an `NVERELOC` buffer.
    ///
    /// # Safety
    /// Caller guarantees `bytes` is an aligned, valid `NVERELOC` buffer produced by [`RelocWriter`].
    #[inline]
    pub unsafe fn from_bytes_unchecked(bytes: &'a [u8]) -> Self {
        // SAFETY: Upheld by caller.
        let header = unsafe { &*bytes.as_ptr().cast::<RelocHeader>() };
        Self {
            bytes,
            header,
            mode: VerifyMode::Unchecked,
        }
    }

    #[inline]
    pub fn header(&self) -> &'a RelocHeader {
        self.header
    }

    #[inline]
    pub fn as_bytes(&self) -> &'a [u8] {
        self.bytes
    }

    #[inline]
    pub fn mode(&self) -> VerifyMode {
        self.mode
    }

    /// Recomputes and checks the XXH64 checksum across the full buffer.
    #[inline]
    pub fn verify_checksum(&self) -> Result<(), RelocError> {
        let actual = compute_reloc_checksum(self.bytes);
        if actual != self.header.checksum {
            return Err(RelocError::ChecksumMismatch {
                expected: self.header.checksum,
                actual,
            });
        }
        Ok(())
    }

    #[inline]
    pub fn require_schema(&self, expected: u32) -> Result<(), RelocError> {
        if self.header.schema_version != expected {
            return Err(RelocError::SchemaVersionMismatch {
                expected,
                actual: self.header.schema_version,
            });
        }
        Ok(())
    }

    #[inline]
    pub fn require_content_kind(&self, expected: u32) -> Result<(), RelocError> {
        if self.header.content_kind != expected {
            return Err(RelocError::ContentKindMismatch {
                expected,
                actual: self.header.content_kind,
            });
        }
        Ok(())
    }

    #[inline]
    fn check_span<T: RelocPod>(&self, offset: usize, count: usize) -> Result<usize, RelocError> {
        let align = align_of::<T>();
        let addr = (self.bytes.as_ptr() as usize)
            .checked_add(offset)
            .ok_or(RelocError::Overflow)?;
        if !offset.is_multiple_of(align) || !addr.is_multiple_of(align) {
            return Err(RelocError::Misaligned {
                expected: align,
                actual: addr,
            });
        }
        let byte_len = count
            .checked_mul(size_of::<T>())
            .ok_or(RelocError::Overflow)?;
        let end = offset.checked_add(byte_len).ok_or(RelocError::Overflow)?;
        if offset < RELOC_HEADER_SIZE || end > self.bytes.len() {
            return Err(RelocError::OutOfBounds("typed span"));
        }
        Ok(end)
    }

    #[inline]
    pub fn root<T: RelocPod>(&self) -> Result<&'a T, RelocError> {
        self.get(RelocPtr::new(self.header.root_offset))
    }

    #[inline]
    pub fn get<T: RelocPod>(&self, ptr: RelocPtr<T>) -> Result<&'a T, RelocError> {
        if ptr.is_null() {
            return Err(RelocError::NullPointer);
        }
        let off = ptr.offset as usize;
        self.check_span::<T>(off, 1)?;
        // SAFETY: Bounds and alignment checked above; `T: RelocPod` has no invalid bit patterns.
        Ok(unsafe { &*self.bytes.as_ptr().add(off).cast::<T>() })
    }

    #[inline]
    pub fn slice<T: RelocPod>(&self, s: RelocSlice<T>) -> Result<&'a [T], RelocError> {
        if s.is_empty() {
            return Ok(&[]);
        }
        let off = s.offset as usize;
        let len = s.len as usize;
        self.check_span::<T>(off, len)?;
        // SAFETY: Bounds and alignment checked above; `T: RelocPod`.
        Ok(unsafe { std::slice::from_raw_parts(self.bytes.as_ptr().add(off).cast::<T>(), len) })
    }

    #[inline]
    pub fn bytes_at(&self, s: RelocSlice<u8>) -> Result<&'a [u8], RelocError> {
        self.slice::<u8>(s)
    }

    #[inline]
    pub fn str_at(&self, s: RelocStr) -> Result<&'a str, RelocError> {
        if s.is_empty() {
            return Ok("");
        }
        let start = s.offset as usize;
        let end = start
            .checked_add(s.len as usize)
            .ok_or(RelocError::Overflow)?;
        if start < RELOC_HEADER_SIZE || end > self.bytes.len() {
            return Err(RelocError::OutOfBounds("reloc str"));
        }
        std::str::from_utf8(&self.bytes[start..end]).map_err(|_| RelocError::InvalidUtf8)
    }

    /// Resolves a [`RelocStr`] after UTF-8 was already validated.
    ///
    /// # Safety
    /// Bytes at `s` must be valid UTF-8.
    #[inline]
    pub unsafe fn str_at_unchecked(&self, s: RelocStr) -> Result<&'a str, RelocError> {
        if s.is_empty() {
            return Ok("");
        }
        let start = s.offset as usize;
        let end = start
            .checked_add(s.len as usize)
            .ok_or(RelocError::Overflow)?;
        if start < RELOC_HEADER_SIZE || end > self.bytes.len() {
            return Err(RelocError::OutOfBounds("reloc str"));
        }
        // SAFETY: Upheld by caller.
        Ok(unsafe { std::str::from_utf8_unchecked(&self.bytes[start..end]) })
    }
}

/// Owned memory-mapped `NVERELOC` file handle backed by [`ReadOnlyMmap`].
pub struct MmapReloc {
    mmap: ReadOnlyMmap,
    mode: VerifyMode,
}

impl MmapReloc {
    pub fn open(path: impl AsRef<Path>, mode: VerifyMode) -> Result<Self, RelocError> {
        let mmap = ReadOnlyMmap::open(path.as_ref())?;
        let effective = match mode {
            VerifyMode::Unchecked => VerifyMode::HeaderOnly,
            other => other,
        };
        let _ = RelocView::from_bytes(mmap.as_slice(), effective)?;
        Ok(Self {
            mmap,
            mode: effective,
        })
    }

    pub fn open_with_schema(
        path: impl AsRef<Path>,
        expected_schema: u32,
        mode: VerifyMode,
    ) -> Result<Self, RelocError> {
        let mapped = Self::open(path, mode)?;
        mapped.view().require_schema(expected_schema)?;
        Ok(mapped)
    }

    /// Opens and maps an `NVERELOC` file with zero-check pointer casting (`VerifyMode::Unchecked`).
    ///
    /// # Safety
    /// File contents must be a valid `NVERELOC` container produced by [`RelocWriter`].
    pub unsafe fn open_unchecked(path: impl AsRef<Path>) -> Result<Self, RelocError> {
        let mmap = ReadOnlyMmap::open(path.as_ref())?;
        if mmap.len() < RELOC_HEADER_SIZE {
            return Err(RelocError::Truncated);
        }
        Ok(Self {
            mmap,
            mode: VerifyMode::Unchecked,
        })
    }

    #[inline]
    pub fn view(&self) -> RelocView<'_> {
        // SAFETY: `open` already validated the header & checksum per `self.mode`.
        let mut v = unsafe { RelocView::from_bytes_unchecked(self.mmap.as_slice()) };
        v.mode = self.mode;
        v
    }

    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        self.mmap.as_slice()
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.mmap.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.mmap.len() == 0
    }
}

/// Zero-copy view of a [`RelocBundleRoot`] (`RELOC_KIND_BUNDLE`) containing Frame-0, state, and metadata.
#[derive(Clone, Copy)]
pub struct RelocBundleView<'a> {
    view: RelocView<'a>,
    frame0: &'a [u8],
    state: &'a [u8],
    meta: &'a [u8],
}

impl<'a> RelocBundleView<'a> {
    pub fn from_bytes(bytes: &'a [u8], mode: VerifyMode) -> Result<Self, RelocError> {
        let view = RelocView::from_bytes(bytes, mode)?;
        Self::from_view(view)
    }

    pub fn from_view(view: RelocView<'a>) -> Result<Self, RelocError> {
        view.require_content_kind(RELOC_KIND_BUNDLE)?;
        let root = view.root::<RelocBundleRoot>()?;
        if root.reserved != 0 {
            return Err(RelocError::InvalidReservedHeader);
        }
        let frame0 = view.bytes_at(root.frame0)?;
        let state = view.bytes_at(root.state)?;
        let meta = view.bytes_at(root.meta)?;
        Ok(Self {
            view,
            frame0,
            state,
            meta,
        })
    }

    #[inline]
    pub fn header(&self) -> &'a RelocHeader {
        self.view.header()
    }

    #[inline]
    pub fn frame0(&self) -> &'a [u8] {
        self.frame0
    }

    #[inline]
    pub fn state(&self) -> &'a [u8] {
        self.state
    }

    #[inline]
    pub fn meta(&self) -> &'a [u8] {
        self.meta
    }
}

/// Encodes a Frame-0 + state + metadata bundle into a single `NVERELOC` container
/// matching C++ `nve::encode_snapshot` byte-for-byte.
pub fn encode_reloc_bundle(
    schema_version: u32,
    flags: u32,
    frame0: &[u8],
    state: &[u8],
    meta: &[u8],
) -> Result<RelocBuf, RelocError> {
    let cap = size_of::<RelocBundleRoot>() + frame0.len() + state.len() + meta.len() + 32;
    let mut w = RelocWriter::with_capacity(RELOC_KIND_BUNDLE, schema_version, flags, cap);
    let root_ptr = w.write_val(&RelocBundleRoot::default())?;
    let frame0_slice = w.write_bytes(frame0, 8)?;
    let state_slice = w.write_bytes(state, 8)?;
    let meta_slice = w.write_bytes(meta, 8)?;

    w.patch_val(
        root_ptr,
        &RelocBundleRoot {
            frame0: frame0_slice,
            state: state_slice,
            meta: meta_slice,
            reserved: 0,
        },
    );
    w.finish(root_ptr)
}

/// Crash-safely writes an `NVERELOC` byte buffer via sibling temp file + `sync_all()` + atomic rename.
pub fn write_reloc_atomic(path: impl AsRef<Path>, bytes: &[u8]) -> Result<(), RelocError> {
    let dest = path.as_ref();
    if let Some(parent) = dest.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }

    let file_name = dest
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("frame0.nvereloc");
    let seq = ATOMIC_TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp_name = format!(".{file_name}.tmp.{}.{seq}", current_pid());
    let tmp_path = match dest.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.join(&tmp_name),
        _ => PathBuf::from(&tmp_name),
    };

    let write_res = (|| -> std::io::Result<()> {
        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp_path)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        Ok(())
    })();

    if let Err(err) = write_res {
        let _ = fs::remove_file(&tmp_path);
        return Err(RelocError::Io(err));
    }

    if let Err(err) = fs::rename(&tmp_path, dest) {
        let _ = fs::remove_file(&tmp_path);
        return Err(RelocError::Io(err));
    }

    #[cfg(unix)]
    if let Some(parent) = dest.parent() {
        let dir = if parent.as_os_str().is_empty() {
            Path::new(".")
        } else {
            parent
        };
        let _ = fs::File::open(dir).and_then(|d| d.sync_all());
    }

    Ok(())
}
