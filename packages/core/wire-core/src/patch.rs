//! Compact binary `NodePatchDelta` payload codec built on 24-byte `#[repr(C)]` [`NveNodePatch`].
//!
//! Wire layout (little-endian):
//! - `patch_count: u32` (0..4)
//! - `string_table_len: u32` (4..8)
//! - `patches: [NveNodePatch; patch_count]` (`24 * patch_count` bytes)
//! - `string_table: [u8; string_table_len]` (UTF-8 bytes referenced by `NveNodePatch::value_lo/hi`)

use std::mem::{align_of, offset_of, size_of};

use crate::frame::NveWireError;

/// Flag bit on [`NveNodePatch::flags`] indicating `(value_lo, value_hi)` stores a UTF-8
/// `(offset, len)` span inside the enclosing `NodePatchDelta` string table.
pub const NVE_NODE_PATCH_FLAG_STRING_SPAN: u16 = 1 << 0;

/// Discriminator identifying which visual or text field an [`NveNodePatch`] updates.
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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
    pub const fn from_u16(raw: u16) -> Option<Self> {
        match raw {
            1 => Some(Self::Text),
            2 => Some(Self::BackgroundRgba),
            3 => Some(Self::TextRgba),
            4 => Some(Self::BorderRgba),
            5 => Some(Self::Opacity),
            6 => Some(Self::FontSize),
            7 => Some(Self::Visibility),
            8 => Some(Self::RectX),
            9 => Some(Self::RectY),
            10 => Some(Self::RectWidth),
            11 => Some(Self::RectHeight),
            12 => Some(Self::ZIndex),
            13 => Some(Self::BorderRadius),
            14 => Some(Self::BorderWidth),
            15 => Some(Self::ImageSrc),
            16 => Some(Self::Tag),
            17 => Some(Self::CustomScalar),
            _ => None,
        }
    }

    #[inline]
    pub const fn is_string_field(self) -> bool {
        matches!(self, Self::Text | Self::ImageSrc | Self::Tag)
    }
}

/// Compact 24-byte `#[repr(C)]` single-field mutation for a scene node.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NveNodePatch {
    pub node_id: u32,
    pub field_tag: u16,
    pub flags: u16,
    /// Scalar value (`u32`, `f32::to_bits`, or low 32 bits) or UTF-8 string byte offset.
    pub value_lo: u32,
    /// Secondary scalar value (high 32 bits) or UTF-8 string byte length.
    pub value_hi: u32,
    /// Optional spring/easing transition hint in milliseconds (`0` = immediate).
    pub transition_ms: u32,
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
        node_id: u32,
        field: NveNodePatchField,
        value_lo: u32,
        value_hi: u32,
        transition_ms: u32,
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
        node_id: u32,
        field: NveNodePatchField,
        str_offset: u32,
        str_len: u32,
        transition_ms: u32,
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
        NveNodePatchField::from_u16(self.field_tag)
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

    pub fn set_text(&mut self, node_id: u32, text: &str) -> &mut Self {
        self.push_string(node_id, NveNodePatchField::Text, text, 0)
    }

    pub fn set_image_src(&mut self, node_id: u32, src: &str) -> &mut Self {
        self.push_string(node_id, NveNodePatchField::ImageSrc, src, 0)
    }

    pub fn set_bg_rgba(&mut self, node_id: u32, rgba: u32) -> &mut Self {
        self.patches.push(NveNodePatch::scalar(
            node_id,
            NveNodePatchField::BackgroundRgba,
            rgba,
            0,
            0,
        ));
        self
    }

    pub fn set_text_rgba(&mut self, node_id: u32, rgba: u32) -> &mut Self {
        self.patches.push(NveNodePatch::scalar(
            node_id,
            NveNodePatchField::TextRgba,
            rgba,
            0,
            0,
        ));
        self
    }

    pub fn set_opacity(&mut self, node_id: u32, opacity: f32, transition_ms: u32) -> &mut Self {
        self.patches.push(NveNodePatch::scalar(
            node_id,
            NveNodePatchField::Opacity,
            opacity.to_bits(),
            0,
            transition_ms,
        ));
        self
    }

    pub fn set_visible(&mut self, node_id: u32, visible: bool) -> &mut Self {
        self.patches.push(NveNodePatch::scalar(
            node_id,
            NveNodePatchField::Visibility,
            u32::from(!visible),
            0,
            0,
        ));
        self
    }

    pub fn set_rect_width(&mut self, node_id: u32, width: f32, transition_ms: u32) -> &mut Self {
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
        node_id: u32,
        field: NveNodePatchField,
        value: &str,
        transition_ms: u32,
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
        if self.patches.len() > MAX_PATCHES_PER_DELTA {
            return Err(NveWireError::InvalidPayload("patch count exceeds 65536"));
        }
        let total = NVE_NODE_PATCH_DELTA_HEADER_SIZE
            + self.patches.len() * PATCH_BYTE_SIZE
            + self.strings.len();
        let mut out = Vec::with_capacity(total);
        out.extend_from_slice(&(self.patches.len() as u32).to_le_bytes());
        out.extend_from_slice(&(self.strings.len() as u32).to_le_bytes());
        for p in &self.patches {
            out.extend_from_slice(&p.node_id.to_le_bytes());
            out.extend_from_slice(&p.field_tag.to_le_bytes());
            out.extend_from_slice(&p.flags.to_le_bytes());
            out.extend_from_slice(&p.value_lo.to_le_bytes());
            out.extend_from_slice(&p.value_hi.to_le_bytes());
            out.extend_from_slice(&p.transition_ms.to_le_bytes());
            out.extend_from_slice(&p.reserved.to_le_bytes());
        }
        out.extend_from_slice(&self.strings);
        Ok(out)
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
        if bytes.len() < NVE_NODE_PATCH_DELTA_HEADER_SIZE {
            return Err(NveWireError::Truncated {
                expected: NVE_NODE_PATCH_DELTA_HEADER_SIZE,
                actual: bytes.len(),
            });
        }
        let patch_count = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
        let string_table_len =
            u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as usize;
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
        let mut patches = Vec::with_capacity(patch_count);
        for i in 0..patch_count {
            let off = NVE_NODE_PATCH_DELTA_HEADER_SIZE + i * PATCH_BYTE_SIZE;
            let le16 = |o: usize| u16::from_le_bytes([bytes[o], bytes[o + 1]]);
            let le32 = |o: usize| {
                u32::from_le_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]])
            };
            let p = NveNodePatch {
                node_id: le32(off),
                field_tag: le16(off + 4),
                flags: le16(off + 6),
                value_lo: le32(off + 8),
                value_hi: le32(off + 12),
                transition_ms: le32(off + 16),
                reserved: le32(off + 20),
            };
            if p.field().is_none() || p.reserved != 0 {
                return Err(NveWireError::InvalidPayload("invalid NveNodePatch field or reserved"));
            }
            if (p.flags & NVE_NODE_PATCH_FLAG_STRING_SPAN) != 0 {
                let end = (p.value_lo as usize)
                    .checked_add(p.value_hi as usize)
                    .ok_or(NveWireError::InvalidPayload("patch string span overflow"))?;
                if end > string_table.len() {
                    return Err(NveWireError::InvalidPayload("patch string span out of bounds"));
                }
                if std::str::from_utf8(&string_table[p.value_lo as usize..end]).is_err() {
                    return Err(NveWireError::InvalidUtf8);
                }
            }
            patches.push(p);
        }

        Ok(Self {
            patches,
            string_table,
        })
    }

    #[inline]
    pub fn patches(&self) -> &[NveNodePatch] {
        &self.patches
    }

    #[inline]
    pub fn patch_str(&self, patch: &NveNodePatch) -> Result<&'a str, NveWireError> {
        if !patch.is_string_span() {
            return Err(NveWireError::InvalidPayload("patch is not a string span"));
        }
        let start = patch.value_lo as usize;
        let end = start + patch.value_hi as usize;
        std::str::from_utf8(&self.string_table[start..end]).map_err(|_| NveWireError::InvalidUtf8)
    }
}
