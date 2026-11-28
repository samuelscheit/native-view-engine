//! Epoch-tagged [`accesskit::NodeId`] encoding and validation preventing stale assistive-technology
//! actions from targeting recycled nodes across Tier-2 hot reloads or keyframe resets.

use accesskit::NodeId as AccessKitNodeId;
use nve_store::NodeId;

/// Bit 63 marks synthetic `Role::TextRun` child nodes owned by a `TextInput` node.
const TEXT_RUN_BIT: u64 = 1u64 << 63;
/// 31-bit non-zero store epoch stored in bits `32..63`.
const EPOCH_MASK: u64 = 0x7FFF_FFFF;
const EPOCH_MAX: u32 = 0x7FFF_FFFF;

/// Monotonic non-zero generation of the backing [`nve_store::NodeStore`].
///
/// Advances whenever the host replaces the store (e.g. Tier-2 `DylibReload` or `frame == 1`
/// keyframe reset) so queued assistive-technology [`accesskit::ActionRequest`]s from an earlier
/// generation are rejected instead of aliasing newly created nodes.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StoreEpoch(u32);

impl StoreEpoch {
    /// Initial non-zero store epoch.
    pub const INITIAL: Self = Self(1);

    /// Constructs a validated non-zero epoch (clamping `0` to `1` and masking to 31 bits).
    #[inline]
    pub const fn new(raw: u32) -> Self {
        let masked = raw & EPOCH_MAX;
        if masked == 0 { Self(1) } else { Self(masked) }
    }

    /// Returns the raw 31-bit non-zero epoch value.
    #[inline]
    pub const fn raw(self) -> u32 {
        self.0
    }

    /// Advances to the next non-zero epoch (wrapping from `0x7FFF_FFFF` back to `1`).
    #[inline]
    pub const fn next(self) -> Self {
        let n = self.0.wrapping_add(1) & EPOCH_MAX;
        if n == 0 { Self(1) } else { Self(n) }
    }
}

impl Default for StoreEpoch {
    fn default() -> Self {
        Self::INITIAL
    }
}

/// Decoded [`AccessKitNodeId`] belonging to the current [`StoreEpoch`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DecodedA11yId {
    /// Primary node corresponding directly to a [`NodeId`].
    Node(NodeId),
    /// Synthetic `Role::TextRun` child owned by a `TextInput` [`NodeId`].
    TextRun(NodeId),
}

impl DecodedA11yId {
    /// Returns the owning [`NodeId`] regardless of whether this ID is the primary node or
    /// its synthetic `TextRun` child.
    #[inline]
    pub const fn owner(self) -> NodeId {
        match self {
            Self::Node(id) | Self::TextRun(id) => id,
        }
    }
}

/// Encodes `(epoch, id)` as a primary [`AccessKitNodeId`] (bit 63 = `0`).
#[inline]
pub const fn encode_store_node_id(epoch: StoreEpoch, id: NodeId) -> AccessKitNodeId {
    let raw = ((epoch.raw() as u64 & EPOCH_MASK) << 32) | (id.raw() as u64);
    AccessKitNodeId(raw)
}

/// Encodes `(epoch, owner)` as the synthetic `Role::TextRun` [`AccessKitNodeId`] (bit 63 = `1`).
#[inline]
pub const fn encode_text_run_id(epoch: StoreEpoch, owner: NodeId) -> AccessKitNodeId {
    let raw = TEXT_RUN_BIT | ((epoch.raw() as u64 & EPOCH_MASK) << 32) | (owner.raw() as u64);
    AccessKitNodeId(raw)
}

/// Decodes `id` if and only if its embedded epoch matches `current_epoch` and its low 32 bits form
/// a non-null [`NodeId`].
#[inline]
pub const fn decode_a11y_id(
    id: AccessKitNodeId,
    current_epoch: StoreEpoch,
) -> Option<DecodedA11yId> {
    let bits = id.0;
    let is_text_run = (bits & TEXT_RUN_BIT) != 0;
    let epoch = ((bits >> 32) & EPOCH_MASK) as u32;
    let low = (bits & 0xFFFF_FFFF) as u32;
    if epoch != current_epoch.raw() {
        return None;
    }
    let Some(nid) = NodeId::from_raw(low) else {
        return None;
    };
    if is_text_run {
        Some(DecodedA11yId::TextRun(nid))
    } else {
        Some(DecodedA11yId::Node(nid))
    }
}

/// Decodes a primary [`NodeId`], returning `None` for stale epochs, null IDs, or synthetic
/// `TextRun` IDs.
#[inline]
pub const fn decode_store_node_id(
    id: AccessKitNodeId,
    current_epoch: StoreEpoch,
) -> Option<NodeId> {
    match decode_a11y_id(id, current_epoch) {
        Some(DecodedA11yId::Node(nid)) => Some(nid),
        Some(DecodedA11yId::TextRun(_)) | None => None,
    }
}
