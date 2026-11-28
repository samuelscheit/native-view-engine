//! Compact binary `NodePatchDelta` payload codec built on 24-byte `#[repr(C)]` [`NveNodePatch`].
//!
//! Wire layout (little-endian):
//! - `patch_count: u32` (0..4)
//! - `string_table_len: u32` (4..8)
//! - `patches: [NveNodePatch; patch_count]` (`24 * patch_count` bytes)
//! - `string_table: [u8; string_table_len]` (UTF-8 bytes referenced by `NveNodePatch::value_lo/hi`)

use std::mem::{align_of, offset_of, size_of};

use nve_abi::{DurationMs, NodeId};
use nve_codec::{ByteReader, ByteWriter};

use crate::frame::NveWireError;

/// Flag bit on [`NveNodePatch::flags`] indicating `(value_lo, value_hi)` stores a UTF-8
/// `(offset, len)` span inside the enclosing `NodePatchDelta` string table.
pub const NVE_NODE_PATCH_FLAG_STRING_SPAN: u16 = 1 << 0;

/// Discriminator identifying which visual or text field an [`NveNodePatch`] updates.
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, nve_codec::ReprEnum)]
pub enum NveNodePatchField {
    Text = 1,
    BackgroundRgba = 2,
    TextRgba = 3,
    BorderRgba = 4,
    Opacity = 5,
    FontSize = 6,
    Visibility = 7,
    RectX = 8,
    RectY = 9,
    RectWidth = 10,
    RectHeight = 11,
    ZIndex = 12,
    BorderRadius = 13,
    BorderWidth = 14,
    ImageSrc = 15,
    Tag = 16,
    CustomScalar = 17,
}

impl NveNodePatchField {
    #[inline]
    pub const fn is_string_field(self) -> bool {
        matches!(self, Self::Text | Self::ImageSrc | Self::Tag)
    }
}

/// Compact 24-byte `#[repr(C)]` single-field mutation for a scene node.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NveNodePatch {
    pub node_id: NodeId,
    pub field_tag: u16,
    pub flags: u16,
    /// Scalar value (`u32`, `f32::to_bits`, or low 32 bits) or UTF-8 string byte offset.
    pub value_lo: u32,
    /// Secondary scalar value (high 32 bits) or UTF-8 string byte length.
    pub value_hi: u32,
    /// Optional spring/easing transition hint in milliseconds (`0` = immediate).
    pub transition_ms: DurationMs,
    pub reserved: u32,
}

const _: () = {
    assert!(size_of::<NveNodePatch>() == 24);
    assert!(align_of::<NveNodePatch>() == 4);
    assert!(offset_of!(NveNodePatch, node_id) == 0);
    assert!(offset_of!(NveNodePatch, field_tag) == 4);
    assert!(offset_of!(NveNodePatch, flags) == 6);
    assert!(offset_of!(NveNodePatch, value_lo) == 8);
    assert!(offset_of!(NveNodePatch, value_hi) == 12);
    assert!(offset_of!(NveNodePatch, transition_ms) == 16);
    assert!(offset_of!(NveNodePatch, reserved) == 20);
};

impl NveNodePatch {
    #[inline]
    pub const fn scalar(
        node_id: NodeId,
        field: NveNodePatchField,
        value_lo: u32,
        value_hi: u32,
        transition_ms: DurationMs,
    ) -> Self {
        Self {
            node_id,
            field_tag: field as u16,
            flags: 0,
            value_lo,
            value_hi,
            transition_ms,
            reserved: 0,
        }
    }

    #[inline]
    pub const fn string_span(
        node_id: NodeId,
        field: NveNodePatchField,
        str_offset: u32,
        str_len: u32,
        transition_ms: DurationMs,
    ) -> Self {
        Self {
            node_id,
            field_tag: field as u16,
            flags: NVE_NODE_PATCH_FLAG_STRING_SPAN,
            value_lo: str_offset,
            value_hi: str_len,
            transition_ms,
            reserved: 0,
        }
    }

    #[inline]
    pub const fn field(&self) -> Option<NveNodePatchField> {
        NveNodePatchField::try_from_u16(self.field_tag)
    }

    #[inline]
    pub const fn is_string_span(&self) -> bool {
        (self.flags & NVE_NODE_PATCH_FLAG_STRING_SPAN) != 0
    }

    #[inline]
    pub const fn as_f32(&self) -> f32 {
        f32::from_bits(self.value_lo)
    }

    #[inline]
    pub const fn as_u64(&self) -> u64 {
        (self.value_lo as u64) | ((self.value_hi as u64) << 32)
    }
}

/// Fixed 8-byte subheader for `NveWireKind::NodePatchDelta` payloads.
pub const NVE_NODE_PATCH_DELTA_HEADER_SIZE: usize = 8;

const PATCH_BYTE_SIZE: usize = 24;
const MAX_PATCHES_PER_DELTA: usize = 65_536;

/// Builder for compact `NodePatchDelta` payloads (`< 512 B` for realistic 8-node live updates).
#[derive(Debug, Default, Clone)]
pub struct NveNodePatchDeltaBuilder {
    patches: Vec<NveNodePatch>,
    strings: Vec<u8>,
}

impl NveNodePatchDeltaBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_capacity(patch_count: usize, string_bytes: usize) -> Self {
        Self {
            patches: Vec::with_capacity(patch_count),
            strings: Vec::with_capacity(string_bytes),
        }
    }

    pub fn len(&self) -> usize {
        self.patches.len()
    }

    pub fn is_empty(&self) -> bool {
        self.patches.is_empty()
    }

    pub fn set_text(&mut self, node_id: NodeId, text: &str) -> &mut Self {
        self.push_string(node_id, NveNodePatchField::Text, text, DurationMs::ZERO)
    }

    pub fn set_image_src(&mut self, node_id: NodeId, src: &str) -> &mut Self {
        self.push_string(node_id, NveNodePatchField::ImageSrc, src, DurationMs::ZERO)
    }

    pub fn set_bg_rgba(&mut self, node_id: NodeId, rgba: u32) -> &mut Self {
        self.patches.push(NveNodePatch::scalar(
            node_id,
            NveNodePatchField::BackgroundRgba,
            rgba,
            0,
            DurationMs::ZERO,
        ));
        self
    }

    pub fn set_text_rgba(&mut self, node_id: NodeId, rgba: u32) -> &mut Self {
        self.patches.push(NveNodePatch::scalar(
            node_id,
            NveNodePatchField::TextRgba,
            rgba,
            0,
            DurationMs::ZERO,
        ));
        self
    }

    pub fn set_opacity(
        &mut self,
        node_id: NodeId,
        opacity: f32,
        transition_ms: DurationMs,
    ) -> &mut Self {
        self.patches.push(NveNodePatch::scalar(
            node_id,
            NveNodePatchField::Opacity,
            opacity.to_bits(),
            0,
            transition_ms,
        ));
        self
    }

    pub fn set_visible(&mut self, node_id: NodeId, visible: bool) -> &mut Self {
        self.patches.push(NveNodePatch::scalar(
            node_id,
            NveNodePatchField::Visibility,
            u32::from(!visible),
            0,
            DurationMs::ZERO,
        ));
        self
    }

    pub fn set_rect_width(
        &mut self,
        node_id: NodeId,
        width: f32,
        transition_ms: DurationMs,
    ) -> &mut Self {
        self.patches.push(NveNodePatch::scalar(
            node_id,
            NveNodePatchField::RectWidth,
            width.to_bits(),
            0,
            transition_ms,
        ));
        self
    }

    pub fn push_string(
        &mut self,
        node_id: NodeId,
        field: NveNodePatchField,
        value: &str,
        transition_ms: DurationMs,
    ) -> &mut Self {
        let offset = self.strings.len() as u32;
        let len = value.len() as u32;
        self.strings.extend_from_slice(value.as_bytes());
        self.patches.push(NveNodePatch::string_span(
            node_id,
            field,
            offset,
            len,
            transition_ms,
        ));
        self
    }

    pub fn push_raw(&mut self, patch: NveNodePatch) -> &mut Self {
        self.patches.push(patch);
        self
    }

    pub fn encode(&self) -> Result<Vec<u8>, NveWireError> {
        let total = NVE_NODE_PATCH_DELTA_HEADER_SIZE
            + self.patches.len().saturating_mul(PATCH_BYTE_SIZE)
            + self.strings.len();
        let mut out = Vec::with_capacity(total);
        self.encode_into(&mut out)?;
        Ok(out)
    }

    /// Encodes the `NodePatchDelta` payload into `out` (clearing `out` first) with zero heap
    /// allocations once `out` has sufficient capacity.
    pub fn encode_into(&self, out: &mut Vec<u8>) -> Result<(), NveWireError> {
        if self.patches.len() > MAX_PATCHES_PER_DELTA {
            return Err(NveWireError::InvalidPayload("patch count exceeds 65536"));
        }
        let total = NVE_NODE_PATCH_DELTA_HEADER_SIZE
            + self.patches.len() * PATCH_BYTE_SIZE
            + self.strings.len();
        out.clear();
        ByteWriter::with_buf(out, |w| {
            w.reserve(total);
            w.write_u32(self.patches.len() as u32);
            w.write_u32(self.strings.len() as u32);
            for p in &self.patches {
                w.write_u32(p.node_id.raw());
                w.write_u16(p.field_tag);
                w.write_u16(p.flags);
                w.write_u32(p.value_lo);
                w.write_u32(p.value_hi);
                w.write_u32(p.transition_ms.raw());
                w.write_u32(p.reserved);
            }
            w.write_raw(&self.strings);
        });
        Ok(())
    }
}

/// Validated view over a `NodePatchDelta` payload.
#[derive(Debug, Clone)]
pub struct NveNodePatchDeltaView<'a> {
    patches: Vec<NveNodePatch>,
    string_table: &'a [u8],
}

impl<'a> NveNodePatchDeltaView<'a> {
    pub fn decode(bytes: &'a [u8]) -> Result<Self, NveWireError> {
        let mut patches = Vec::new();
        let string_table = Self::decode_into(bytes, &mut patches)?;
        Ok(Self {
            patches,
            string_table,
        })
    }

    /// Decodes and validates a `NodePatchDelta` payload into a reusable `out` buffer (clearing it
    /// first) with zero heap allocations once `out` has sufficient capacity.
    ///
    /// Returns the validated UTF-8 string-table slice borrowed from `bytes` for lookups via
    /// [`Self::resolve_patch_str`]. On error, `out` is cleared so partial untrusted patches are
    /// never exposed.
    pub fn decode_into(
        bytes: &'a [u8],
        out: &mut Vec<NveNodePatch>,
    ) -> Result<&'a [u8], NveWireError> {
        out.clear();
        if bytes.len() < NVE_NODE_PATCH_DELTA_HEADER_SIZE {
            return Err(NveWireError::Truncated {
                expected: NVE_NODE_PATCH_DELTA_HEADER_SIZE,
                actual: bytes.len(),
            });
        }
        let mut r = ByteReader::new(bytes);
        let patch_count = r.read_u32()? as usize;
        let string_table_len = r.read_u32()? as usize;
        if patch_count > MAX_PATCHES_PER_DELTA {
            return Err(NveWireError::InvalidPayload("patch count exceeds 65536"));
        }
        let patches_bytes = patch_count
            .checked_mul(PATCH_BYTE_SIZE)
            .ok_or(NveWireError::InvalidPayload("patch byte size overflow"))?;
        let expected_len = NVE_NODE_PATCH_DELTA_HEADER_SIZE
            .checked_add(patches_bytes)
            .and_then(|v| v.checked_add(string_table_len))
            .ok_or(NveWireError::InvalidPayload("patch payload length overflow"))?;
        if bytes.len() < expected_len {
            return Err(NveWireError::Truncated {
                expected: expected_len,
                actual: bytes.len(),
            });
        }
        if bytes.len() > expected_len {
            return Err(NveWireError::TrailingBytes(bytes.len() - expected_len));
        }

        let str_start = NVE_NODE_PATCH_DELTA_HEADER_SIZE + patches_bytes;
        let string_table = &bytes[str_start..];
        out.reserve(patch_count);
        for _ in 0..patch_count {
            let Some(node_id) = NodeId::from_raw(r.read_u32()?) else {
                out.clear();
                return Err(NveWireError::InvalidPayload("null node_id in NveNodePatch"));
            };
            let p = NveNodePatch {
                node_id,
                field_tag: r.read_u16()?,
                flags: r.read_u16()?,
                value_lo: r.read_u32()?,
                value_hi: r.read_u32()?,
                transition_ms: DurationMs(r.read_u32()?),
                reserved: r.read_u32()?,
            };
            if p.field().is_none() || p.reserved != 0 {
                out.clear();
                return Err(NveWireError::InvalidPayload(
                    "invalid NveNodePatch field or reserved",
                ));
            }
            if (p.flags & NVE_NODE_PATCH_FLAG_STRING_SPAN) != 0 {
                let Some(end) = (p.value_lo as usize).checked_add(p.value_hi as usize) else {
                    out.clear();
                    return Err(NveWireError::InvalidPayload("patch string span overflow"));
                };
                if end > string_table.len() {
                    out.clear();
                    return Err(NveWireError::InvalidPayload("patch string span out of bounds"));
                }
                if std::str::from_utf8(&string_table[p.value_lo as usize..end]).is_err() {
                    out.clear();
                    return Err(NveWireError::InvalidUtf8);
                }
            }
            out.push(p);
        }

        Ok(string_table)
    }

    #[inline]
    pub fn patches(&self) -> &[NveNodePatch] {
        &self.patches
    }

    #[inline]
    pub fn patch_str(&self, patch: &NveNodePatch) -> Result<&'a str, NveWireError> {
        Self::resolve_patch_str(self.string_table, patch)
    }

    /// Resolves the UTF-8 string referenced by `patch` within a validated `string_table` slice
    /// returned by [`Self::decode_into`].
    #[inline]
    pub fn resolve_patch_str(
        string_table: &'a [u8],
        patch: &NveNodePatch,
    ) -> Result<&'a str, NveWireError> {
        if !patch.is_string_span() {
            return Err(NveWireError::InvalidPayload("patch is not a string span"));
        }
        let start = patch.value_lo as usize;
        let end = start
            .checked_add(patch.value_hi as usize)
            .ok_or(NveWireError::InvalidPayload("patch string span overflow"))?;
        let slice = string_table
            .get(start..end)
            .ok_or(NveWireError::InvalidPayload("patch string span out of bounds"))?;
        std::str::from_utf8(slice).map_err(|_| NveWireError::InvalidUtf8)
    }
}
