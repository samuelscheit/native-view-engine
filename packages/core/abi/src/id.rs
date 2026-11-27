//! Strongly typed `#[repr(transparent)]` ABI handles and scalar unit newtypes shared across
//! `nve-abi`, `nve-store`, `nve-engine`, `nve-interop`, backends, modules, and C/foreign hosts.
//!
//! Every handle has the exact size, alignment, and register calling convention of its underlying
//! primitive (`u8`/`u16`/`u32`/`i32`/`u64`) while preventing accidental mixing of domain spaces
//! (`NodeId` vs `SurfaceId` vs `NodeIndex` vs `FrameId` vs `DurationUs`) at compile time.

use core::fmt;
use core::mem::{align_of, size_of};
use core::ops::{BitAnd, BitAndAssign, BitOr, BitOrAssign};

use nve_codec::NveCodec;

/// Dense slot index into `NodeStore`'s node table (`0` is reserved for null).
///
/// Distinct from [`NodeId`] (`index:24 | generation:8`) so internal slot-index lists cannot be
/// accidentally mixed with generational `NodeId`s at compile time.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, NveCodec)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct NodeIndex(pub u32);

impl NodeIndex {
    pub const NULL: Self = Self(0);
    pub const ROOT: Self = Self(1);

    #[inline(always)]
    pub const fn as_usize(self) -> usize {
        self.0 as usize
    }

    #[inline(always)]
    pub const fn raw(self) -> u32 {
        self.0
    }
}

impl fmt::Display for NodeIndex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Generational node identity: `index: 24 | generation: 8`. Raw `0` is [`NodeId::NULL`].
///
/// Indices are recycled only at frame boundaries; the 8-bit generation catches stale IDs across
/// frames without hash maps or heap allocations.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, NveCodec)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct NodeId(pub u32);

impl NodeId {
    pub const NULL: Self = Self(0);
    /// Reserved root node of every surface (index `1`, generation `0`).
    pub const ROOT: Self = Self(1);
    pub const INDEX_BITS: u32 = 24;
    pub const MAX_INDEX: u32 = (1 << Self::INDEX_BITS) - 1;

    #[inline(always)]
    pub const fn new(index: u32, generation: u8) -> Self {
        Self(((generation as u32) << Self::INDEX_BITS) | (index & Self::MAX_INDEX))
    }

    #[inline(always)]
    pub const fn from_index(index: NodeIndex, generation: u8) -> Self {
        Self::new(index.0, generation)
    }

    #[inline(always)]
    pub const fn node_index(self) -> NodeIndex {
        NodeIndex(self.0 & Self::MAX_INDEX)
    }

    #[inline(always)]
    pub const fn index(self) -> usize {
        (self.0 & Self::MAX_INDEX) as usize
    }

    #[inline(always)]
    pub const fn index_u32(self) -> u32 {
        self.0 & Self::MAX_INDEX
    }

    #[inline(always)]
    pub const fn generation(self) -> u8 {
        (self.0 >> Self::INDEX_BITS) as u8
    }

    #[inline(always)]
    pub const fn is_null(self) -> bool {
        self.0 == 0
    }

    #[inline(always)]
    pub const fn raw(self) -> u32 {
        self.0
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Reserved root node ID for every surface (`NodeId::ROOT`).
pub const ROOT_NODE_ID: NodeId = NodeId::ROOT;

/// Generational surface identity: `index: 24 | generation: 8`. Raw `0` is [`SurfaceId::NULL`].
///
/// Raw FFI structs keep `SurfaceId` as a plain `u32` (never `NonZeroU32` inside `extern "C"` POD
/// so zero-initialized C buffers never trigger UB); ingress validation rejects [`SurfaceId::NULL`].
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, NveCodec)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct SurfaceId(pub u32);

impl SurfaceId {
    pub const NULL: Self = Self(0);
    /// Conventional first surface (index `1`, generation `0`).
    pub const ROOT: Self = Self(1);
    pub const INDEX_BITS: u32 = 24;
    pub const MAX_INDEX: u32 = (1 << Self::INDEX_BITS) - 1;

    #[inline(always)]
    pub const fn new(index: u32, generation: u8) -> Self {
        Self(((generation as u32) << Self::INDEX_BITS) | (index & Self::MAX_INDEX))
    }

    #[inline(always)]
    pub const fn index(self) -> usize {
        (self.0 & Self::MAX_INDEX) as usize
    }

    #[inline(always)]
    pub const fn index_u32(self) -> u32 {
        self.0 & Self::MAX_INDEX
    }

    #[inline(always)]
    pub const fn generation(self) -> u8 {
        (self.0 >> Self::INDEX_BITS) as u8
    }

    #[inline(always)]
    pub const fn is_null(self) -> bool {
        self.0 == 0
    }

    #[inline(always)]
    pub const fn raw(self) -> u32 {
        self.0
    }
}

impl fmt::Display for SurfaceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Monotonic 64-bit frame identifier.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, NveCodec)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct FrameId(pub u64);

impl FrameId {
    pub const ZERO: Self = Self(0);

    #[inline(always)]
    pub const fn raw(self) -> u64 {
        self.0
    }

    #[inline(always)]
    pub const fn next(self) -> Self {
        Self(self.0.wrapping_add(1))
    }
}

impl fmt::Display for FrameId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Monotonic timestamp in microseconds.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, NveCodec)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct TimestampUs(pub u64);

impl TimestampUs {
    pub const ZERO: Self = Self(0);

    #[inline(always)]
    pub const fn raw(self) -> u64 {
        self.0
    }
}

impl fmt::Display for TimestampUs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Elapsed duration in microseconds.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, NveCodec)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct DurationUs(pub u64);

impl DurationUs {
    pub const ZERO: Self = Self(0);

    #[inline(always)]
    pub const fn raw(self) -> u64 {
        self.0
    }
}

impl fmt::Display for DurationUs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Elapsed duration in milliseconds.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, NveCodec)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct DurationMs(pub u32);

impl DurationMs {
    pub const ZERO: Self = Self(0);

    #[inline(always)]
    pub const fn raw(self) -> u32 {
        self.0
    }
}

impl fmt::Display for DurationMs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Unix timestamp in milliseconds.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, NveCodec)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct TimestampMs(pub u64);

impl TimestampMs {
    pub const ZERO: Self = Self(0);

    #[inline(always)]
    pub const fn raw(self) -> u64 {
        self.0
    }
}

impl fmt::Display for TimestampMs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Table-scoped event discriminant (`0..32`).
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, NveCodec)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct EventId(pub u32);

impl EventId {
    #[inline(always)]
    pub const fn raw(self) -> u32 {
        self.0
    }

    #[inline(always)]
    pub const fn as_usize(self) -> usize {
        self.0 as usize
    }
}

impl fmt::Display for EventId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Typed bitflags on a uniform 64-byte `NveEvent` (modifiers, pointer buttons, capture, IME).
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, NveCodec)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct EventFlags(pub u32);

impl EventFlags {
    pub const EMPTY: Self = Self(0);

    #[inline(always)]
    pub const fn raw(self) -> u32 {
        self.0
    }

    #[inline(always)]
    pub const fn contains(self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }

    #[inline(always)]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    #[inline(always)]
    pub const fn intersection(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }
}

impl BitOr for EventFlags {
    type Output = Self;
    #[inline(always)]
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl BitOrAssign for EventFlags {
    #[inline(always)]
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

impl BitAnd for EventFlags {
    type Output = Self;
    #[inline(always)]
    fn bitand(self, rhs: Self) -> Self {
        Self(self.0 & rhs.0)
    }
}

impl BitAndAssign for EventFlags {
    #[inline(always)]
    fn bitand_assign(&mut self, rhs: Self) {
        self.0 &= rhs.0;
    }
}

/// Byte offset inside a flat arena or event batch byte buffer.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, NveCodec)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct ArenaOffset(pub u32);

impl ArenaOffset {
    pub const ZERO: Self = Self(0);

    #[inline(always)]
    pub const fn raw(self) -> u32 {
        self.0
    }

    #[inline(always)]
    pub const fn as_usize(self) -> usize {
        self.0 as usize
    }
}

/// 32-bit byte or element length across ABI headers and arenas.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, NveCodec)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct ByteLen(pub u32);

impl ByteLen {
    pub const ZERO: Self = Self(0);

    #[inline(always)]
    pub const fn raw(self) -> u32 {
        self.0
    }

    #[inline(always)]
    pub const fn as_usize(self) -> usize {
        self.0 as usize
    }
}

/// Compact 16-bit byte length inside fixed-stride 64-byte scene records.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, NveCodec)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct ShortLen(pub u16);

impl ShortLen {
    pub const ZERO: Self = Self(0);

    #[inline(always)]
    pub const fn raw(self) -> u16 {
        self.0
    }

    #[inline(always)]
    pub const fn as_usize(self) -> usize {
        self.0 as usize
    }
}

/// Realizing table slot index on an `NveRenderNode` (`0..128`), or [`PrimarySlot::NONE`] (`0xFF`)
/// for a plain container node.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, NveCodec)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct PrimarySlot(pub u8);

impl PrimarySlot {
    pub const NONE: Self = Self(0xFF);

    #[inline(always)]
    pub const fn raw(self) -> u8 {
        self.0
    }

    #[inline(always)]
    pub const fn is_none(self) -> bool {
        self.0 == 0xFF
    }

    #[inline(always)]
    pub const fn slot(self) -> Option<u8> {
        if self.0 == 0xFF {
            None
        } else {
            Some(self.0)
        }
    }
}

impl Default for PrimarySlot {
    #[inline(always)]
    fn default() -> Self {
        Self::NONE
    }
}

/// Stacking z-index layer order.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, NveCodec)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct ZIndex(pub i32);

impl ZIndex {
    pub const ZERO: Self = Self(0);

    #[inline(always)]
    pub const fn raw(self) -> i32 {
        self.0
    }
}

/// Modular C ABI interface identifier passed to `nve_get_interface`.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, NveCodec)]
pub struct InterfaceId(pub u32);

impl InterfaceId {
    #[inline(always)]
    pub const fn raw(self) -> u32 {
        self.0
    }
}

/// Direct prop field index in `NvePropEntry`.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, NveCodec)]
pub struct PropFieldId(pub u16);

impl PropFieldId {
    #[inline(always)]
    pub const fn raw(self) -> u16 {
        self.0
    }
}

const _: () = {
    assert!(size_of::<NodeIndex>() == 4 && align_of::<NodeIndex>() == 4);
    assert!(size_of::<NodeId>() == 4 && align_of::<NodeId>() == 4);
    assert!(size_of::<SurfaceId>() == 4 && align_of::<SurfaceId>() == 4);
    assert!(size_of::<FrameId>() == 8 && align_of::<FrameId>() == 8);
    assert!(size_of::<TimestampUs>() == 8 && align_of::<TimestampUs>() == 8);
    assert!(size_of::<DurationUs>() == 8 && align_of::<DurationUs>() == 8);
    assert!(size_of::<DurationMs>() == 4 && align_of::<DurationMs>() == 4);
    assert!(size_of::<TimestampMs>() == 8 && align_of::<TimestampMs>() == 8);
    assert!(size_of::<EventId>() == 4 && align_of::<EventId>() == 4);
    assert!(size_of::<EventFlags>() == 4 && align_of::<EventFlags>() == 4);
    assert!(size_of::<ArenaOffset>() == 4 && align_of::<ArenaOffset>() == 4);
    assert!(size_of::<ByteLen>() == 4 && align_of::<ByteLen>() == 4);
    assert!(size_of::<ShortLen>() == 2 && align_of::<ShortLen>() == 2);
    assert!(size_of::<PrimarySlot>() == 1 && align_of::<PrimarySlot>() == 1);
    assert!(size_of::<ZIndex>() == 4 && align_of::<ZIndex>() == 4);
    assert!(size_of::<InterfaceId>() == 4 && align_of::<InterfaceId>() == 4);
    assert!(size_of::<PropFieldId>() == 2 && align_of::<PropFieldId>() == 2);
};
