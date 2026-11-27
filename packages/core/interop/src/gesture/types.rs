//! Data types and inline small-buffer collections for cross-framework gesture arbitration.

use nve_abi::hash::HashMap;
use nve_abi::{SurfaceId, TimestampUs};
use serde::{Deserialize, Serialize};

use crate::surface::{FrameworkKind, SurfaceOrchestrator};

pub(super) const INLINE_CHAIN_CAP: usize = SurfaceOrchestrator::INLINE_ANCESTOR_CAP;

/// Zero-dependency inline small-buffer surface ancestor chain.
/// Stores up to `INLINE_CHAIN_CAP` (32) nested surfaces inline with zero heap allocation,
/// spilling to `Vec` only on unusually deep trees (`len > 32`).
#[derive(Debug, Clone)]
pub(super) enum SurfaceChain {
    Inline {
        len: u8,
        buf: [SurfaceId; INLINE_CHAIN_CAP],
    },
    Heap(Vec<SurfaceId>),
}

impl SurfaceChain {
    #[inline]
    pub(super) fn from_slice(slice: &[SurfaceId]) -> Self {
        if slice.len() <= INLINE_CHAIN_CAP {
            let mut buf = [SurfaceId::NULL; INLINE_CHAIN_CAP];
            buf[..slice.len()].copy_from_slice(slice);
            Self::Inline {
                len: slice.len() as u8,
                buf,
            }
        } else {
            Self::Heap(slice.to_vec())
        }
    }

    #[inline(always)]
    pub(super) fn as_slice(&self) -> &[SurfaceId] {
        match self {
            Self::Inline { len, buf } => &buf[..usize::from(*len)],
            Self::Heap(v) => v.as_slice(),
        }
    }

    #[inline(always)]
    pub(super) fn is_empty(&self) -> bool {
        match self {
            Self::Inline { len, .. } => *len == 0,
            Self::Heap(v) => v.is_empty(),
        }
    }

    pub(super) fn retain<F: FnMut(SurfaceId) -> bool>(&mut self, mut keep: F) {
        match self {
            Self::Inline { len, buf } => {
                let mut write = 0usize;
                let n = usize::from(*len);
                for read in 0..n {
                    let id = buf[read];
                    if keep(id) {
                        buf[write] = id;
                        write += 1;
                    }
                }
                *len = write as u8;
            }
            Self::Heap(v) => v.retain(|&id| keep(id)),
        }
    }
}

/// Fast key-indexed map optimized for typical small counts (<= `N` entries stored inline with
/// zero heap allocation and cache-local linear key scan) with seamless spill to an Fx-hashed
/// `HashMap` for larger tables.
#[derive(Debug)]
pub(super) enum FastKeyMap<K, V, const N: usize> {
    Inline {
        len: u8,
        keys: [K; N],
        vals: [Option<V>; N],
    },
    Spill(HashMap<K, V>),
}

impl<K: Copy + Default, V, const N: usize> Default for FastKeyMap<K, V, N> {
    #[inline]
    fn default() -> Self {
        Self::Inline {
            len: 0,
            keys: [K::default(); N],
            vals: std::array::from_fn(|_| None),
        }
    }
}

impl<K: Copy + Eq + std::hash::Hash, V, const N: usize> FastKeyMap<K, V, N> {
    #[inline(always)]
    pub(super) fn len(&self) -> usize {
        match self {
            Self::Inline { len, .. } => usize::from(*len),
            Self::Spill(map) => map.len(),
        }
    }

    #[inline(always)]
    pub(super) fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[inline(always)]
    pub(super) fn get(&self, key: K) -> Option<&V> {
        match self {
            Self::Inline { len, keys, vals } => {
                let n = usize::from(*len);
                let mut i = 0;
                while i < n {
                    if keys[i] == key {
                        return vals[i].as_ref();
                    }
                    i += 1;
                }
                None
            }
            Self::Spill(map) => map.get(&key),
        }
    }

    #[inline(always)]
    pub(super) fn get_mut(&mut self, key: K) -> Option<&mut V> {
        match self {
            Self::Inline { len, keys, vals } => {
                let n = usize::from(*len);
                let mut i = 0;
                while i < n {
                    if keys[i] == key {
                        return vals[i].as_mut();
                    }
                    i += 1;
                }
                None
            }
            Self::Spill(map) => map.get_mut(&key),
        }
    }

    #[inline]
    pub(super) fn insert(&mut self, key: K, val: V) -> Option<V> {
        match self {
            Self::Inline { len, keys, vals } => {
                let n = usize::from(*len);
                let mut i = 0;
                while i < n {
                    if keys[i] == key {
                        return vals[i].replace(val);
                    }
                    i += 1;
                }
                if n < N {
                    keys[n] = key;
                    vals[n] = Some(val);
                    *len = (n + 1) as u8;
                    return None;
                }
                // Spill inline entries to FxHashMap.
                let mut map = HashMap::with_capacity_and_hasher(N * 2, Default::default());
                for idx in 0..n {
                    if let Some(existing) = vals[idx].take() {
                        map.insert(keys[idx], existing);
                    }
                }
                map.insert(key, val);
                *self = Self::Spill(map);
                None
            }
            Self::Spill(map) => map.insert(key, val),
        }
    }

    #[inline]
    pub(super) fn remove(&mut self, key: K) -> Option<V> {
        match self {
            Self::Inline { len, keys, vals } => {
                let n = usize::from(*len);
                let mut i = 0;
                while i < n {
                    if keys[i] == key {
                        let removed = vals[i].take();
                        let last = n - 1;
                        if i != last {
                            keys[i] = keys[last];
                            vals[i] = vals[last].take();
                        }
                        *len = last as u8;
                        return removed;
                    }
                    i += 1;
                }
                None
            }
            Self::Spill(map) => map.remove(&key),
        }
    }

    pub(super) fn retain<F: FnMut(K, &mut V) -> bool>(&mut self, mut keep: F) {
        match self {
            Self::Inline { len, keys, vals } => {
                let mut i = 0usize;
                while i < usize::from(*len) {
                    let should_keep = if let Some(v) = vals[i].as_mut() {
                        keep(keys[i], v)
                    } else {
                        false
                    };
                    if should_keep {
                        i += 1;
                    } else {
                        let last = usize::from(*len) - 1;
                        vals[i] = None;
                        if i != last {
                            keys[i] = keys[last];
                            vals[i] = vals[last].take();
                        }
                        *len = last as u8;
                    }
                }
            }
            Self::Spill(map) => map.retain(|&k, v| keep(k, v)),
        }
    }
}

/// Outcome of hit-testing / pointer arbitration for a surface across framework boundaries.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GestureDisposition {
    /// Surface owns this pointer sequence; competing parents/children must yield.
    Claim = 1,
    /// Surface does not handle this gesture (e.g. orthogonal scroll axis or transparent hit).
    PassThrough = 2,
    /// Surface could handle this gesture in general, but is yielding control to an ancestor
    /// (e.g. inner scrollable reached its scroll boundary and hands off overscroll to outer).
    DeferToParent = 3,
}

/// Phase of a pointer / touch interaction.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PointerPhase {
    Down = 1,
    Move = 2,
    Up = 3,
    Cancel = 4,
}

/// Scroll axes a surface can actively consume.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ScrollAxes {
    pub horizontal: bool,
    pub vertical: bool,
}

impl ScrollAxes {
    pub const NONE: Self = Self {
        horizontal: false,
        vertical: false,
    };
    pub const VERTICAL: Self = Self {
        horizontal: false,
        vertical: true,
    };
    pub const HORIZONTAL: Self = Self {
        horizontal: true,
        vertical: false,
    };
    pub const BOTH: Self = Self {
        horizontal: true,
        vertical: true,
    };

    pub fn any(self) -> bool {
        self.horizontal || self.vertical
    }
}

/// Boundary state of a scrollable surface, used to determine when to defer to a parent scrollable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ScrollEdgeState {
    pub at_top: bool,
    pub at_bottom: bool,
    pub at_left: bool,
    pub at_right: bool,
}

impl ScrollEdgeState {
    /// Returns whether the scrollable can consume a finger drag `(delta_x, delta_y)`.
    ///
    /// Convention (standard touch dragging):
    /// - `delta_y > 0` (finger drags down) scrolls toward the top -> blocked if `at_top`.
    /// - `delta_y < 0` (finger drags up) scrolls toward the bottom -> blocked if `at_bottom`.
    /// - `delta_x > 0` (finger drags right) scrolls toward the left -> blocked if `at_left`.
    /// - `delta_x < 0` (finger drags left) scrolls toward the right -> blocked if `at_right`.
    pub fn can_scroll_vertical(self, delta_y: f32) -> bool {
        if delta_y > 0.0 {
            !self.at_top
        } else if delta_y < 0.0 {
            !self.at_bottom
        } else {
            true
        }
    }

    pub fn can_scroll_horizontal(self, delta_x: f32) -> bool {
        if delta_x > 0.0 {
            !self.at_left
        } else if delta_x < 0.0 {
            !self.at_right
        } else {
            true
        }
    }
}

/// Declared gesture and scroll capabilities of a registered surface.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GestureCapabilities {
    pub framework: FrameworkKind,
    pub handles_tap: bool,
    pub scroll_axes: ScrollAxes,
    pub scroll_edges: ScrollEdgeState,
    /// When `true`, hitting a scroll boundary defers drag to the nearest capable ancestor.
    pub nested_scroll_enabled: bool,
    /// Minimum drag distance (logical px) before classifying a pointer as a scroll/drag.
    pub touch_slop_px: f32,
}

impl Default for GestureCapabilities {
    fn default() -> Self {
        Self {
            framework: FrameworkKind::Nve,
            handles_tap: true,
            scroll_axes: ScrollAxes::NONE,
            scroll_edges: ScrollEdgeState::default(),
            nested_scroll_enabled: true,
            touch_slop_px: 8.0,
        }
    }
}

impl GestureCapabilities {
    pub fn vertical_scroll(framework: FrameworkKind) -> Self {
        Self {
            framework,
            handles_tap: true,
            scroll_axes: ScrollAxes::VERTICAL,
            scroll_edges: ScrollEdgeState::default(),
            nested_scroll_enabled: true,
            touch_slop_px: 8.0,
        }
    }

    pub fn horizontal_scroll(framework: FrameworkKind) -> Self {
        Self {
            framework,
            handles_tap: true,
            scroll_axes: ScrollAxes::HORIZONTAL,
            scroll_edges: ScrollEdgeState::default(),
            nested_scroll_enabled: true,
            touch_slop_px: 8.0,
        }
    }

    pub fn pass_through(framework: FrameworkKind) -> Self {
        Self {
            framework,
            handles_tap: false,
            scroll_axes: ScrollAxes::NONE,
            scroll_edges: ScrollEdgeState::default(),
            nested_scroll_enabled: true,
            touch_slop_px: 8.0,
        }
    }
}

/// Incoming pointer sample submitted to [`super::GestureArbitrator`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PointerSample {
    pub pointer_id: u32,
    pub phase: PointerPhase,
    pub x: f32,
    pub y: f32,
    pub delta_x: f32,
    pub delta_y: f32,
    pub timestamp_us: TimestampUs,
}

/// Why the arbitrator selected a disposition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArbitrationReason {
    InitialTouchDown,
    WithinTouchSlop,
    ExplicitOverride,
    ScrollAxisClaimed,
    OrthogonalAxisPassThrough,
    ScrollBoundaryDeferredToParent,
    ScrollBoundaryConsumedLocally,
    LockedWinnerContinued,
    PointerEnded,
    NoEligibleSurface,
}

/// Result of arbitrating a pointer sample across an innermost-to-outermost surface chain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArbitrationDecision {
    /// Disposition from the perspective of the innermost hit surface.
    pub innermost_disposition: GestureDisposition,
    /// The surface currently owning the pointer gesture, if any.
    pub winner: Option<SurfaceId>,
    /// Surfaces that previously owned or were tentatively tracking this pointer and must receive `Cancel`.
    pub cancelled_surfaces: Vec<SurfaceId>,
    pub reason: ArbitrationReason,
}
