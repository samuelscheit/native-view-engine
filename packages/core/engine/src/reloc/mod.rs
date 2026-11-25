//! Production zero-copy, zero-dependency relocatable binary container (`NVERELOC`).
//!
//! Stores flat `#[repr(C)]` POD structures, slices, and UTF-8 strings addressable directly from
//! `mmap` or 64-byte-aligned [`RelocBuf`] memory with zero deserialization and zero heap
//! allocations on read:
//!
//! - [`RelocHeader`] (64 bytes, 1 CPU cache line, XXH64 integrity checksum)
//! - Generic base-relative handles: [`RelocPtr<T>`], [`RelocSlice<T>`], [`RelocStr`]
//! - Single-pass builder: [`RelocWriter`] -> 64-byte-aligned [`RelocBuf`]
//! - Zero-copy views: [`RelocView<'a>`], [`MmapReloc`], [`RelocBundleView<'a>`]
//! - Pre-rendered Frame-0 scene format: [`SceneRelocView<'a>`] / [`encode_scene_reloc`]
//!
//! Three explicit cold-load validation modes ([`VerifyMode`]):
//! - [`VerifyMode::Full`]: Untrusted validation (header + full XXH64 checksum + payload/UTF-8).
//! - [`VerifyMode::HeaderOnly`]: O(1) header + alignment + root bounds check without faulting pages.
//! - [`VerifyMode::Unchecked`]: Zero-check direct pointer cast (`unsafe` precondition: valid buffer).

pub mod format;
pub mod scene;
#[cfg(test)]
mod tests;
pub mod timeline;
pub mod view;

use std::fmt;
use std::io;

pub use crate::mmap_arena::ArenaPod as RelocPod;
pub use format::{
    compute_reloc_checksum, RelocBuf, RelocBundleRoot, RelocHeader, RelocPtr, RelocSlice, RelocStr,
    RelocWriter, RELOC_BUF_ALIGN, RELOC_HEADER_SIZE, RELOC_KIND_BUNDLE, RELOC_KIND_GENERIC,
    RELOC_KIND_SCENE, RELOC_KIND_TIMELINE, RELOC_KIND_WORKSPACE, RELOC_MAGIC, RELOC_VERSION,
};
pub use scene::{
    encode_scene_reloc, RelocLayoutProps, RelocSceneNode, RelocSceneRoot, SceneRelocInput,
    SceneRelocNodeInput, SceneRelocView, SCENE_NODE_CLIP_CHILDREN, SCENE_NODE_HAS_EXPLICIT_STYLE,
    SCENE_NODE_HIDDEN, SCENE_NODE_INTERACTIVE,
};
pub use timeline::{
    encode_timeline_archive, NveNodePatch, NveNodePatchField, NveNodePatchValue,
    NveTimelineArchive, NveTimelineArchiveView, NveTimelineEntry, NveTimelineEntryInput,
    NveTimelinePatchInput, NveTimelineRoot, NVE_NODE_PATCH_FLAG_STRING_SPAN,
};
pub use view::{encode_reloc_bundle, write_reloc_atomic, MmapReloc, RelocBundleView, RelocView};

/// Self-documenting alias for a 64-byte-aligned zero-copy scene archive (`RelocBuf`).
pub type NveSceneArchive = RelocBuf;
/// Self-documenting alias for a borrowed zero-copy scene archive view (`SceneRelocView`).
pub type NveSceneArchiveView<'a> = SceneRelocView<'a>;

/// Validation tier for cold-loading an `NVERELOC` zero-copy buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifyMode {
    /// Untrusted validation: header, alignment, XXH64 checksum, and structural/UTF-8 invariants.
    Full,
    /// Trusted local `mmap`: O(1) header, alignment, and root bounds checks only.
    HeaderOnly,
    /// Zero-check direct pointer cast (caller upholds layout & UTF-8 invariants).
    Unchecked,
}

/// Errors produced when building, mapping, validating, or reading an `NVERELOC` container.
#[derive(Debug)]
pub enum RelocError {
    Io(io::Error),
    Truncated,
    InvalidMagic,
    UnsupportedVersion(u32),
    InvalidReservedHeader,
    SchemaVersionMismatch { expected: u32, actual: u32 },
    ContentKindMismatch { expected: u32, actual: u32 },
    Misaligned { expected: usize, actual: usize },
    PayloadLengthMismatch { header: u32, actual: usize },
    ChecksumMismatch { expected: u64, actual: u64 },
    NullPointer,
    OutOfBounds(&'static str),
    InvalidUtf8,
    InvalidScene(&'static str),
    Overflow,
}

impl fmt::Display for RelocError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "NVERELOC io error: {e}"),
            Self::Truncated => write!(f, "truncated NVERELOC buffer"),
            Self::InvalidMagic => write!(f, "invalid NVERELOC magic"),
            Self::UnsupportedVersion(v) => write!(f, "unsupported NVERELOC version {v}"),
            Self::InvalidReservedHeader => write!(f, "non-zero reserved bytes in NVERELOC header"),
            Self::SchemaVersionMismatch { expected, actual } => write!(
                f,
                "NVERELOC schema version mismatch (expected {expected}, got {actual})"
            ),
            Self::ContentKindMismatch { expected, actual } => write!(
                f,
                "NVERELOC content_kind mismatch (expected {expected}, got {actual})"
            ),
            Self::Misaligned { expected, actual } => write!(
                f,
                "misaligned NVERELOC address/offset (need align {expected}, got {actual:#x})"
            ),
            Self::PayloadLengthMismatch { header, actual } => write!(
                f,
                "NVERELOC payload_len mismatch (header={header}, actual={actual})"
            ),
            Self::ChecksumMismatch { expected, actual } => write!(
                f,
                "NVERELOC XXH64 checksum mismatch (expected={expected:#018x}, actual={actual:#018x})"
            ),
            Self::NullPointer => write!(f, "null RelocPtr dereference"),
            Self::OutOfBounds(what) => write!(f, "NVERELOC out of bounds: {what}"),
            Self::InvalidUtf8 => write!(f, "invalid UTF-8 in NVERELOC string"),
            Self::InvalidScene(msg) => write!(f, "invalid NVERELOC scene: {msg}"),
            Self::Overflow => write!(f, "NVERELOC offset/size overflow"),
        }
    }
}

impl std::error::Error for RelocError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for RelocError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}
