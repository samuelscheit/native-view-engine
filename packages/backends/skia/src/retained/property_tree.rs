//! Flat, strictly `Copy` property trees (`TransformNode`, `ClipNode`, `EffectNode`, and sparse
//! `ScrollPropertyNode`) for [`super::CompositorScene`].
//!
//! Every node struct is plain-old-data with zero heap fields (`[INV:no-noderow-heap-fields]`)
//! addressed via distinct `#[repr(transparent)]` typed handles (`[INV:typed-domain-handles]`).
//! `TransformNode`, `ClipNode`, and `EffectNode` are indexed 1:1 by [`ChunkIdx`]; cold scroll
//! fields live in a sparse `scrolls` table addressed by [`ScrollPropIdx`] so hot `TransformNode`
//! fits within a single 64-byte cache line.

use nve_engine::LayoutRect;
use nve_store::NodeId;
use nve_tables::transform::Transform2D;

/// Dense index into [`super::CompositorScene::chunks`] and the parallel 1:1 property arrays
/// (`transforms`, `clips`, `effects`).
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ChunkIdx(pub u32);

impl ChunkIdx {
    pub const NONE: Self = Self(u32::MAX);

    #[inline]
    pub const fn is_none(self) -> bool {
        self.0 == u32::MAX
    }

    #[inline]
    pub const fn is_some(self) -> bool {
        self.0 != u32::MAX
    }

    #[inline]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

/// Sparse index into [`PropertyTrees::scrolls`] (`NONE` for non-`Scroll` chunks).
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ScrollPropIdx(pub u32);

impl ScrollPropIdx {
    pub const NONE: Self = Self(u32::MAX);

    #[inline]
    pub const fn is_none(self) -> bool {
        self.0 == u32::MAX
    }

    #[inline]
    pub const fn is_some(self) -> bool {
        self.0 != u32::MAX
    }

    #[inline]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

pub(crate) const TF_HAS_TRANSFORM: u8 = 1 << 0;
pub(crate) const TF_IS_STICKY: u8 = 1 << 1;

/// Hot transform, layout origin, and sticky-offset property node (strictly `Copy` POD, <= 64 B).
#[derive(Debug, Clone, Copy)]
pub struct TransformNode {
    pub base_x: f32,
    pub base_y: f32,
    pub width: f32,
    pub height: f32,
    pub sticky_x: f32,
    pub sticky_y: f32,
    pub scroll_idx: ScrollPropIdx,
    pub local_transform: Transform2D,
    pub flags: u8,
}

impl Default for TransformNode {
    fn default() -> Self {
        Self {
            base_x: 0.0,
            base_y: 0.0,
            width: 0.0,
            height: 0.0,
            sticky_x: 0.0,
            sticky_y: 0.0,
            scroll_idx: ScrollPropIdx::NONE,
            local_transform: Transform2D::IDENTITY,
            flags: 0,
        }
    }
}

impl TransformNode {
    #[inline]
    pub const fn has_transform(&self) -> bool {
        (self.flags & TF_HAS_TRANSFORM) != 0
    }

    #[inline]
    pub const fn has_scroll(&self) -> bool {
        self.scroll_idx.is_some()
    }

    #[inline]
    pub const fn is_sticky(&self) -> bool {
        (self.flags & TF_IS_STICKY) != 0
    }

    #[inline]
    pub const fn size_eq(&self, rect: LayoutRect) -> bool {
        self.width.to_bits() == rect.width.to_bits()
            && self.height.to_bits() == rect.height.to_bits()
    }

    #[inline]
    pub fn set_transform(&mut self, tf: Option<Transform2D>) {
        match tf {
            Some(t) if !t.is_identity() => {
                self.local_transform = t;
                self.flags |= TF_HAS_TRANSFORM;
            }
            _ => {
                self.local_transform = Transform2D::IDENTITY;
                self.flags &= !TF_HAS_TRANSFORM;
            }
        }
    }
}

/// Cold scroll geometry & offset state stored sparsely only for `Scroll` containers.
#[derive(Debug, Clone, Copy, Default)]
pub struct ScrollPropertyNode {
    /// Visual scroll shift subtracted from descendants (accounts for `virtual_top_spacer`).
    pub scroll_shift_x: f32,
    pub scroll_shift_y: f32,
    /// Raw clamped scroll offset used to position overlay scrollbar thumbs.
    pub scroll_raw_x: f32,
    pub scroll_raw_y: f32,
    /// Cached content dimensions of the `Scroll` container (O(1) clamp & scrollbar thumbs).
    pub content_w: f32,
    pub content_h: f32,
}

/// Overflow clipping property node (strictly `Copy` POD, 1:1 with [`ChunkIdx`]).
#[derive(Debug, Clone, Copy, Default)]
pub struct ClipNode {
    pub radius: f32,
    pub clips_children: bool,
}

/// Opacity & CSS visibility property node (strictly `Copy` POD, 1:1 with [`ChunkIdx`]).
#[derive(Debug, Clone, Copy)]
pub struct EffectNode {
    pub opacity: f32,
    pub effective_hidden: bool,
}

impl Default for EffectNode {
    fn default() -> Self {
        Self {
            opacity: 1.0,
            effective_hidden: false,
        }
    }
}

const _: () = assert!(std::mem::size_of::<TransformNode>() <= 64);
const _: () = assert!(std::mem::size_of::<ScrollPropertyNode>() == 24);
const _: () = assert!(std::mem::size_of::<ClipNode>() <= 8);
const _: () = assert!(std::mem::size_of::<EffectNode>() <= 8);

/// SoA property-tree storage owned by [`super::CompositorScene`].
#[derive(Debug, Clone, Default)]
pub struct PropertyTrees {
    pub transforms: Vec<TransformNode>,
    pub clips: Vec<ClipNode>,
    pub effects: Vec<EffectNode>,
    /// Sparse scroll state referenced by `TransformNode::scroll_idx`.
    pub scrolls: Vec<ScrollPropertyNode>,
    /// Live nodes with `Position::Sticky`, refreshed in O(#sticky) when any scroll offset changes.
    pub sticky_chunks: Vec<(NodeId, ChunkIdx)>,
}

impl PropertyTrees {
    pub fn clear(&mut self) {
        self.transforms.clear();
        self.clips.clear();
        self.effects.clear();
        self.scrolls.clear();
        self.sticky_chunks.clear();
    }

    /// Appends one parallel slot across transform, clip, and effect trees (1:1 with `ChunkIdx`).
    #[inline]
    pub fn push_slot(&mut self, tf: TransformNode, clip: ClipNode, effect: EffectNode) {
        self.transforms.push(tf);
        self.clips.push(clip);
        self.effects.push(effect);
    }

    #[inline]
    pub fn reset_slot(&mut self, cidx: ChunkIdx) {
        let i = cidx.index();
        self.transforms[i] = TransformNode::default();
        self.clips[i] = ClipNode::default();
        self.effects[i] = EffectNode::default();
    }

    /// Appends a [`ScrollPropertyNode`] and returns its sparse [`ScrollPropIdx`].
    #[inline]
    pub fn push_scroll(&mut self, scroll: ScrollPropertyNode) -> ScrollPropIdx {
        let idx = ScrollPropIdx(self.scrolls.len() as u32);
        self.scrolls.push(scroll);
        idx
    }
}
