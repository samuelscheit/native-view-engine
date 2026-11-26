//! Zero-copy immutable [`NveTimelineArchive`] pairing one base [`SceneRelocView`] with
//! timestamp-sorted [`NveTimelineEntry`] slices of compact 24-byte [`NveNodePatch`] updates.
//!
//! Designed for iOS WidgetKit / Live Activities (`< 4 KB` APNs push budget), Android Glance
//! `RemoteViews`, watch faces, and offline scheduled UI timelines without re-rendering or JSON.

use std::mem::{align_of, offset_of, size_of};

use super::format::{RelocBuf, RelocSlice, RelocStr, RelocWriter, RELOC_BUF_ALIGN, RELOC_KIND_TIMELINE};
use super::scene::SceneRelocView;
use super::view::RelocView;
use super::{RelocError, RelocPod, VerifyMode};

/// Flag bit on [`NveNodePatch::flags`] indicating `(value_lo, value_hi)` stores a UTF-8
/// `(offset, len)` span inside the enclosing archive or `NodePatchDelta` string table.
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
///
/// Used both zero-copy inside [`NveTimelineArchive`] and directly in `NveWireKind::NodePatchDelta`
/// frames so live score/status patches stay well under Apple's `< 4 KB` Live Activity APNs budget.
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
    pub const fn as_reloc_str(&self) -> RelocStr {
        RelocStr::new(self.value_lo, self.value_hi)
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

/// One scheduled point in an [`NveTimelineArchive`], sorted ascending by `activate_at_unix_ms`.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NveTimelineEntry {
    pub activate_at_unix_ms: u64,
    pub patches: RelocSlice<NveNodePatch>,
}

/// Root descriptor inside an [`NveTimelineArchive`] (`content_kind = RELOC_KIND_TIMELINE`).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NveTimelineRoot {
    /// Embedded 64-byte-aligned base `NveSceneArchive` (`SceneRelocView`) bytes.
    pub base_scene: RelocSlice<u8>,
    /// Entries sorted in non-decreasing `activate_at_unix_ms` order.
    pub entries: RelocSlice<NveTimelineEntry>,
    pub created_at_unix_ms: u64,
    /// Optional archive expiry timestamp (`0` = never expires).
    pub expires_at_unix_ms: u64,
}

// SAFETY: `NveNodePatch` is `#[repr(C)]` (24 bytes) with fixed-width fields and zero implicit padding.
unsafe impl RelocPod for NveNodePatch {}
// SAFETY: `NveTimelineEntry` is `#[repr(C)]` with fixed-width fields and zero implicit padding.
unsafe impl RelocPod for NveTimelineEntry {}
// SAFETY: `NveTimelineRoot` is `#[repr(C)]` composed solely of `RelocPod` fields with zero implicit padding.
unsafe impl RelocPod for NveTimelineRoot {}

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

    assert!(size_of::<NveTimelineEntry>() == 16);
    assert!(align_of::<NveTimelineEntry>() == 8);
    assert!(offset_of!(NveTimelineEntry, activate_at_unix_ms) == 0);
    assert!(offset_of!(NveTimelineEntry, patches) == 8);

    assert!(size_of::<NveTimelineRoot>() == 32);
    assert!(align_of::<NveTimelineRoot>() == 8);
    assert!(offset_of!(NveTimelineRoot, base_scene) == 0);
    assert!(offset_of!(NveTimelineRoot, entries) == 8);
    assert!(offset_of!(NveTimelineRoot, created_at_unix_ms) == 16);
    assert!(offset_of!(NveTimelineRoot, expires_at_unix_ms) == 24);
};

/// Owned patch value used when constructing an [`NveTimelineArchive`].
#[derive(Debug, Clone, PartialEq)]
pub enum NveNodePatchValue {
    Text(String),
    Rgba(u32),
    F32(f32),
    Bool(bool),
    I32(i32),
    U64(u64),
}

/// High-level node patch input for [`encode_timeline_archive`].
#[derive(Debug, Clone, PartialEq)]
pub struct NveTimelinePatchInput {
    pub node_id: u32,
    pub field: NveNodePatchField,
    pub value: NveNodePatchValue,
    pub transition_ms: u32,
}

impl NveTimelinePatchInput {
    pub fn text(node_id: u32, text: impl Into<String>) -> Self {
        Self {
            node_id,
            field: NveNodePatchField::Text,
            value: NveNodePatchValue::Text(text.into()),
            transition_ms: 0,
        }
    }

    pub fn bg_rgba(node_id: u32, rgba: u32) -> Self {
        Self {
            node_id,
            field: NveNodePatchField::BackgroundRgba,
            value: NveNodePatchValue::Rgba(rgba),
            transition_ms: 0,
        }
    }

    pub fn text_rgba(node_id: u32, rgba: u32) -> Self {
        Self {
            node_id,
            field: NveNodePatchField::TextRgba,
            value: NveNodePatchValue::Rgba(rgba),
            transition_ms: 0,
        }
    }

    pub fn opacity(node_id: u32, opacity: f32) -> Self {
        Self {
            node_id,
            field: NveNodePatchField::Opacity,
            value: NveNodePatchValue::F32(opacity),
            transition_ms: 0,
        }
    }

    pub fn visible(node_id: u32, visible: bool) -> Self {
        Self {
            node_id,
            field: NveNodePatchField::Visibility,
            value: NveNodePatchValue::Bool(visible),
            transition_ms: 0,
        }
    }

    pub fn with_transition_ms(mut self, transition_ms: u32) -> Self {
        self.transition_ms = transition_ms;
        self
    }
}

/// One timestamped patch batch input for [`encode_timeline_archive`].
#[derive(Debug, Clone, PartialEq)]
pub struct NveTimelineEntryInput {
    pub activate_at_unix_ms: u64,
    pub patches: Vec<NveTimelinePatchInput>,
}

/// Owned 64-byte-aligned immutable timeline archive (`NveSceneArchive` + sorted timed patches).
#[derive(Clone, Debug)]
pub struct NveTimelineArchive {
    buf: RelocBuf,
}

impl NveTimelineArchive {
    /// Builds a validated [`NveTimelineArchive`] from a base `NveSceneArchive` and timed entries.
    ///
    /// Entries are automatically sorted ascending by `activate_at_unix_ms`.
    pub fn build(
        base_scene_bytes: &[u8],
        entries: &[NveTimelineEntryInput],
        created_at_unix_ms: u64,
        expires_at_unix_ms: u64,
        schema_version: u32,
    ) -> Result<Self, RelocError> {
        let buf = encode_timeline_archive(
            base_scene_bytes,
            entries,
            created_at_unix_ms,
            expires_at_unix_ms,
            schema_version,
        )?;
        Ok(Self { buf })
    }

    #[inline]
    pub fn as_slice(&self) -> &[u8] {
        self.buf.as_slice()
    }

    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        self.buf.as_slice()
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
    pub fn into_reloc_buf(self) -> RelocBuf {
        self.buf
    }

    #[inline]
    pub fn view(&self, mode: VerifyMode) -> Result<NveTimelineArchiveView<'_>, RelocError> {
        NveTimelineArchiveView::from_bytes(self.buf.as_slice(), mode)
    }
}

/// Encodes one base `NveSceneArchive` + timed `NveNodePatch` entries into a 64-byte-aligned `RelocBuf`.
pub fn encode_timeline_archive(
    base_scene_bytes: &[u8],
    entries: &[NveTimelineEntryInput],
    created_at_unix_ms: u64,
    expires_at_unix_ms: u64,
    schema_version: u32,
) -> Result<RelocBuf, RelocError> {
    // Validate base scene up front so corrupt scenes never enter a timeline archive.
    let _ = SceneRelocView::from_bytes(base_scene_bytes, VerifyMode::Full)?;

    let mut order: Vec<usize> = (0..entries.len()).collect();
    order.sort_by_key(|&idx| entries[idx].activate_at_unix_ms);

    let est_cap = base_scene_bytes.len() + entries.len() * 64 + 128;
    let mut writer = RelocWriter::with_capacity(RELOC_KIND_TIMELINE, schema_version, 0, est_cap);

    // Embed the base scene at 64-byte alignment so SceneRelocView opens it zero-copy in place.
    let base_scene = writer.write_bytes(base_scene_bytes, RELOC_BUF_ALIGN)?;

    let mut encoded_entries = Vec::with_capacity(order.len());
    for idx in order {
        let entry = &entries[idx];
        let mut encoded_patches = Vec::with_capacity(entry.patches.len());
        for patch in &entry.patches {
            let encoded = match &patch.value {
                NveNodePatchValue::Text(s) => {
                    let span = writer.write_str(s)?;
                    NveNodePatch::string_span(
                        patch.node_id,
                        patch.field,
                        span.offset,
                        span.len,
                        patch.transition_ms,
                    )
                }
                NveNodePatchValue::Rgba(rgba) => NveNodePatch::scalar(
                    patch.node_id,
                    patch.field,
                    *rgba,
                    0,
                    patch.transition_ms,
                ),
                NveNodePatchValue::F32(v) => NveNodePatch::scalar(
                    patch.node_id,
                    patch.field,
                    v.to_bits(),
                    0,
                    patch.transition_ms,
                ),
                NveNodePatchValue::Bool(visible) => NveNodePatch::scalar(
                    patch.node_id,
                    patch.field,
                    u32::from(!*visible),
                    0,
                    patch.transition_ms,
                ),
                NveNodePatchValue::I32(v) => NveNodePatch::scalar(
                    patch.node_id,
                    patch.field,
                    *v as u32,
                    0,
                    patch.transition_ms,
                ),
                NveNodePatchValue::U64(v) => NveNodePatch::scalar(
                    patch.node_id,
                    patch.field,
                    *v as u32,
                    (*v >> 32) as u32,
                    patch.transition_ms,
                ),
            };
            encoded_patches.push(encoded);
        }
        let patches_slice = writer.write_slice(&encoded_patches)?;
        encoded_entries.push(NveTimelineEntry {
            activate_at_unix_ms: entry.activate_at_unix_ms,
            patches: patches_slice,
        });
    }

    let entries_slice = writer.write_slice(&encoded_entries)?;
    let root = writer.write_val(&NveTimelineRoot {
        base_scene,
        entries: entries_slice,
        created_at_unix_ms,
        expires_at_unix_ms,
    })?;
    writer.finish(root)
}

/// Zero-copy borrowed view directly over a validated [`NveTimelineArchive`].
#[derive(Clone, Copy)]
pub struct NveTimelineArchiveView<'a> {
    view: RelocView<'a>,
    root: &'a NveTimelineRoot,
    entries: &'a [NveTimelineEntry],
    base_scene_bytes: &'a [u8],
}

impl<'a> NveTimelineArchiveView<'a> {
    /// Opens and validates an [`NveTimelineArchiveView`] according to `mode`.
    pub fn from_bytes(bytes: &'a [u8], mode: VerifyMode) -> Result<Self, RelocError> {
        let view = RelocView::from_bytes(bytes, mode)?;
        Self::from_view(view)
    }

    /// Wraps an already-opened [`RelocView`] as an [`NveTimelineArchiveView`].
    pub fn from_view(view: RelocView<'a>) -> Result<Self, RelocError> {
        view.require_content_kind(RELOC_KIND_TIMELINE)?;
        let root: &'a NveTimelineRoot = view.root()?;
        let base_scene_bytes = view.slice(root.base_scene)?;
        let entries = view.slice(root.entries)?;

        // Enforce non-decreasing timestamps so binary search is always sound.
        let mut prev_ts = 0u64;
        for (i, entry) in entries.iter().enumerate() {
            if i > 0 && entry.activate_at_unix_ms < prev_ts {
                return Err(RelocError::OutOfBounds("timeline_entries_unsorted"));
            }
            prev_ts = entry.activate_at_unix_ms;
            let patches = view.slice(entry.patches)?;
            if view.mode() == VerifyMode::Full {
                for patch in patches {
                    if patch.field().is_none() || patch.reserved != 0 {
                        return Err(RelocError::OutOfBounds("timeline_patch_field"));
                    }
                    if patch.is_string_span() {
                        let _ = view.str_at(patch.as_reloc_str())?;
                    }
                }
            }
        }

        // Validate base scene header (or full checksum in Full mode).
        let _ = SceneRelocView::from_bytes(base_scene_bytes, view.mode())?;

        Ok(Self {
            view,
            root,
            entries,
            base_scene_bytes,
        })
    }

    #[inline]
    pub fn root(&self) -> &'a NveTimelineRoot {
        self.root
    }

    #[inline]
    pub fn entries(&self) -> &'a [NveTimelineEntry] {
        self.entries
    }

    /// Opens the embedded zero-copy base [`SceneRelocView`].
    #[inline]
    pub fn base_scene(&self) -> Result<SceneRelocView<'a>, RelocError> {
        SceneRelocView::from_bytes(self.base_scene_bytes, VerifyMode::HeaderOnly)
    }

    /// Returns the zero-copy slice of [`NveNodePatch`] entries for `entry`.
    #[inline]
    pub fn patches_for(&self, entry: &NveTimelineEntry) -> Result<&'a [NveNodePatch], RelocError> {
        self.view.slice(entry.patches)
    }

    /// Resolves a UTF-8 string referenced by a string-span [`NveNodePatch`].
    #[inline]
    pub fn patch_str(&self, patch: &NveNodePatch) -> Result<&'a str, RelocError> {
        if !patch.is_string_span() {
            return Err(RelocError::OutOfBounds("patch_not_string_span"));
        }
        self.view.str_at(patch.as_reloc_str())
    }

    /// Binary-searches the latest timeline entry whose `activate_at_unix_ms <= now_unix_ms`.
    #[inline]
    pub fn active_entry_at(&self, now_unix_ms: u64) -> Option<(usize, &'a NveTimelineEntry)> {
        let count = self
            .entries
            .partition_point(|e| e.activate_at_unix_ms <= now_unix_ms);
        if count == 0 {
            None
        } else {
            let idx = count - 1;
            Some((idx, &self.entries[idx]))
        }
    }

    /// Returns the active [`NveNodePatch`] slice at `now_unix_ms`, or an empty slice before the first entry.
    pub fn active_patches_at(&self, now_unix_ms: u64) -> Result<&'a [NveNodePatch], RelocError> {
        match self.active_entry_at(now_unix_ms) {
            Some((_, entry)) => self.patches_for(entry),
            None => Ok(&[]),
        }
    }

    #[inline]
    pub fn is_expired_at(&self, now_unix_ms: u64) -> bool {
        self.root.expires_at_unix_ms != 0 && now_unix_ms >= self.root.expires_at_unix_ms
    }
}

#[cfg(test)]
#[path = "timeline_tests.rs"]
mod tests;
