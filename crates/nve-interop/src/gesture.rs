use std::collections::HashMap;
use std::hash::{BuildHasher, Hasher};

use nve_abi::SurfaceId;
use serde::{Deserialize, Serialize};

use crate::surface::FrameworkKind;

const INLINE_CHAIN_CAP: usize = 16;

/// Zero-dependency inline small-buffer surface ancestor chain.
/// Stores up to 16 nested surfaces inline with zero heap allocation, spilling to `Vec` only on
/// unusually deep trees (`len > 16`).
#[derive(Debug, Clone)]
enum SurfaceChain {
    Inline {
        len: u8,
        buf: [SurfaceId; INLINE_CHAIN_CAP],
    },
    Heap(Vec<SurfaceId>),
}

impl SurfaceChain {
    #[inline]
    fn from_slice(slice: &[SurfaceId]) -> Self {
        if slice.len() <= INLINE_CHAIN_CAP {
            let mut buf = [0u32; INLINE_CHAIN_CAP];
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
    fn as_slice(&self) -> &[SurfaceId] {
        match self {
            Self::Inline { len, buf } => &buf[..usize::from(*len)],
            Self::Heap(v) => v.as_slice(),
        }
    }

    #[inline(always)]
    fn is_empty(&self) -> bool {
        match self {
            Self::Inline { len, .. } => *len == 0,
            Self::Heap(v) => v.is_empty(),
        }
    }

    fn retain<F: FnMut(SurfaceId) -> bool>(&mut self, mut keep: F) {
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

/// Minimal zero-dependency FxHasher for integer keys (`u32` surface/pointer IDs).
#[derive(Default, Clone)]
struct FxHasher {
    hash: u64,
}

impl Hasher for FxHasher {
    #[inline(always)]
    fn finish(&self) -> u64 {
        self.hash
    }

    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.hash = (self.hash.rotate_left(5) ^ u64::from(b))
                .wrapping_mul(0x517c_c1b7_2722_0a95);
        }
    }

    #[inline(always)]
    fn write_u32(&mut self, i: u32) {
        self.hash = (self.hash.rotate_left(5) ^ u64::from(i)).wrapping_mul(0x517c_c1b7_2722_0a95);
    }

    #[inline(always)]
    fn write_u64(&mut self, i: u64) {
        self.hash = (self.hash.rotate_left(5) ^ i).wrapping_mul(0x517c_c1b7_2722_0a95);
    }
}

#[derive(Debug, Default, Clone)]
struct BuildFxHasher;

impl BuildHasher for BuildFxHasher {
    type Hasher = FxHasher;

    #[inline(always)]
    fn build_hasher(&self) -> FxHasher {
        FxHasher { hash: 0 }
    }
}

/// Fast integer-keyed map optimized for typical small counts (<= `N` entries stored inline with
/// zero heap allocation and cache-local linear key scan) with seamless spill to an Fx-hashed
/// `HashMap` for larger tables.
#[derive(Debug)]
enum FastU32Map<V, const N: usize> {
    Inline {
        len: u8,
        keys: [u32; N],
        vals: [Option<V>; N],
    },
    Spill(HashMap<u32, V, BuildFxHasher>),
}

impl<V, const N: usize> Default for FastU32Map<V, N> {
    #[inline]
    fn default() -> Self {
        Self::Inline {
            len: 0,
            keys: [0; N],
            vals: std::array::from_fn(|_| None),
        }
    }
}

impl<V, const N: usize> FastU32Map<V, N> {
    #[inline(always)]
    fn len(&self) -> usize {
        match self {
            Self::Inline { len, .. } => usize::from(*len),
            Self::Spill(map) => map.len(),
        }
    }

    #[inline(always)]
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[inline(always)]
    fn get(&self, key: u32) -> Option<&V> {
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
    fn get_mut(&mut self, key: u32) -> Option<&mut V> {
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
    fn insert(&mut self, key: u32, val: V) -> Option<V> {
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
                let mut map = HashMap::with_capacity_and_hasher(N * 2, BuildFxHasher);
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
    fn remove(&mut self, key: u32) -> Option<V> {
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

    fn retain<F: FnMut(u32, &mut V) -> bool>(&mut self, mut keep: F) {
        match self {
            Self::Inline { len, keys, vals } => {
                let mut i = 0usize;
                while i < usize::from(*len) {
                    let should_keep = {
                        let k = keys[i];
                        let v = vals[i].as_mut().expect("occupied inline map slot");
                        keep(k, v)
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

/// Incoming pointer sample submitted to [`GestureArbitrator`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PointerSample {
    pub pointer_id: u32,
    pub phase: PointerPhase,
    pub x: f32,
    pub y: f32,
    pub delta_x: f32,
    pub delta_y: f32,
    pub timestamp_us: u64,
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

#[derive(Debug, Clone)]
struct ActivePointer {
    chain: SurfaceChain,
    down_x: f32,
    down_y: f32,
    winner: Option<SurfaceId>,
    locked_scroll: bool,
}

/// Cross-framework touch and nested-scroll arbitration engine.
///
/// Solves inner-vs-outer scroll conflicts across framework boundaries (e.g. an NVE vertical
/// `ScrollView` embedded inside a Flutter `ListView`, or a React Native horizontal carousel
/// inside a SwiftUI vertical `ScrollView`).
#[derive(Debug, Default)]
pub struct GestureArbitrator {
    capabilities: FastU32Map<GestureCapabilities, 16>,
    overrides: FastU32Map<GestureDisposition, 8>,
    active_pointers: FastU32Map<ActivePointer, 8>,
}

impl GestureArbitrator {
    pub fn new() -> Self {
        Self::default()
    }

    #[inline]
    pub fn set_capabilities(&mut self, surface_id: SurfaceId, caps: GestureCapabilities) {
        self.capabilities.insert(surface_id, caps);
    }

    #[inline]
    pub fn update_scroll_edges(&mut self, surface_id: SurfaceId, edges: ScrollEdgeState) {
        if let Some(caps) = self.capabilities.get_mut(surface_id) {
            caps.scroll_edges = edges;
        }
    }

    /// Forces an explicit disposition for a surface (e.g. from a custom native hit-test hook).
    #[inline]
    pub fn set_override(&mut self, surface_id: SurfaceId, disposition: Option<GestureDisposition>) {
        match disposition {
            Some(d) => {
                self.overrides.insert(surface_id, d);
            }
            None => {
                self.overrides.remove(surface_id);
            }
        }
    }

    /// Removes all gesture state for a destroyed surface.
    pub fn unregister_surface(&mut self, surface_id: SurfaceId) {
        self.capabilities.remove(surface_id);
        self.overrides.remove(surface_id);
        self.active_pointers.retain(|_, track| {
            track.chain.retain(|id| id != surface_id);
            if track.winner == Some(surface_id) {
                track.winner = None;
                track.locked_scroll = false;
            }
            !track.chain.is_empty()
        });
    }

    #[inline]
    pub fn active_pointer_count(&self) -> usize {
        self.active_pointers.len()
    }

    /// Arbitrates a pointer sample along `hit_chain` (ordered **innermost -> outermost**).
    #[inline]
    pub fn arbitrate(
        &mut self,
        hit_chain: &[SurfaceId],
        sample: PointerSample,
    ) -> ArbitrationDecision {
        if hit_chain.is_empty() {
            if matches!(sample.phase, PointerPhase::Up | PointerPhase::Cancel) {
                self.active_pointers.remove(sample.pointer_id);
            }
            return ArbitrationDecision {
                innermost_disposition: GestureDisposition::PassThrough,
                winner: None,
                cancelled_surfaces: Vec::new(),
                reason: ArbitrationReason::NoEligibleSurface,
            };
        }

        let innermost = hit_chain[0];

        match sample.phase {
            PointerPhase::Down => self.handle_down(hit_chain, innermost, sample),
            PointerPhase::Move => self.handle_move(hit_chain, innermost, sample),
            PointerPhase::Up | PointerPhase::Cancel => self.handle_end(innermost, sample),
        }
    }

    #[inline]
    fn handle_down(
        &mut self,
        hit_chain: &[SurfaceId],
        innermost: SurfaceId,
        sample: PointerSample,
    ) -> ArbitrationDecision {
        if !self.overrides.is_empty()
            && let Some(&forced) = self.overrides.get(innermost)
        {
            let winner = match forced {
                GestureDisposition::Claim => Some(innermost),
                GestureDisposition::DeferToParent => hit_chain.get(1).copied(),
                GestureDisposition::PassThrough => hit_chain
                    .iter()
                    .skip(1)
                    .copied()
                    .find(|id| self.is_interactive(*id)),
            };
            self.active_pointers.insert(
                sample.pointer_id,
                ActivePointer {
                    chain: SurfaceChain::from_slice(hit_chain),
                    down_x: sample.x,
                    down_y: sample.y,
                    winner,
                    locked_scroll: false,
                },
            );
            return ArbitrationDecision {
                innermost_disposition: forced,
                winner,
                cancelled_surfaces: Vec::new(),
                reason: ArbitrationReason::ExplicitOverride,
            };
        }

        let tentative_winner = hit_chain.iter().copied().find(|id| self.is_interactive(*id));
        let innermost_disposition = match tentative_winner {
            Some(id) if id == innermost => GestureDisposition::Claim,
            Some(_) => GestureDisposition::PassThrough,
            None => GestureDisposition::PassThrough,
        };

        self.active_pointers.insert(
            sample.pointer_id,
            ActivePointer {
                chain: SurfaceChain::from_slice(hit_chain),
                down_x: sample.x,
                down_y: sample.y,
                winner: tentative_winner,
                locked_scroll: false,
            },
        );

        ArbitrationDecision {
            innermost_disposition,
            winner: tentative_winner,
            cancelled_surfaces: Vec::new(),
            reason: ArbitrationReason::InitialTouchDown,
        }
    }

    #[inline]
    fn handle_move(
        &mut self,
        hit_chain: &[SurfaceId],
        innermost: SurfaceId,
        sample: PointerSample,
    ) -> ArbitrationDecision {
        let capabilities = &self.capabilities;
        let track_opt = self.active_pointers.get_mut(sample.pointer_id);

        let (chain, down_x, down_y, prev_winner, prev_locked_scroll): (
            &[SurfaceId],
            f32,
            f32,
            Option<SurfaceId>,
            bool,
        ) = if let Some(track) = track_opt.as_deref() {
            (
                track.chain.as_slice(),
                track.down_x,
                track.down_y,
                track.winner,
                track.locked_scroll,
            )
        } else {
            // Synthetic Move without prior Down: borrow hit_chain directly with zero allocation.
            (
                hit_chain,
                sample.x - sample.delta_x,
                sample.y - sample.delta_y,
                None,
                false,
            )
        };

        let total_dx = sample.x - down_x;
        let total_dy = sample.y - down_y;
        let step_dx = if sample.delta_x != 0.0 {
            sample.delta_x
        } else {
            total_dx
        };
        let step_dy = if sample.delta_y != 0.0 {
            sample.delta_y
        } else {
            total_dy
        };

        if !prev_locked_scroll {
            let slop = capabilities
                .get(innermost)
                .map(|c| c.touch_slop_px)
                .unwrap_or(8.0);
            let dist_sq = total_dx * total_dx + total_dy * total_dy;
            if slop > 0.0 && dist_sq < slop * slop {
                let disp = if prev_winner == Some(innermost) {
                    GestureDisposition::Claim
                } else {
                    GestureDisposition::PassThrough
                };
                return ArbitrationDecision {
                    innermost_disposition: disp,
                    winner: prev_winner,
                    cancelled_surfaces: Vec::new(),
                    reason: ArbitrationReason::WithinTouchSlop,
                };
            }
        }

        // Determine dominant scroll axis from step (or total if step is tiny).
        let dominant_horizontal = if step_dx.abs() != step_dy.abs() {
            step_dx.abs() > step_dy.abs()
        } else {
            total_dx.abs() > total_dy.abs()
        };

        // Evaluate innermost-to-outermost for scroll ownership.
        let mut chosen_winner: Option<SurfaceId> = None;
        let mut deferred_inner = false;
        let mut consumed_boundary_locally = false;

        for (idx, &surface_id) in chain.iter().enumerate() {
            let Some(caps) = capabilities.get(surface_id) else {
                continue;
            };
            let supports_axis = if dominant_horizontal {
                caps.scroll_axes.horizontal
            } else {
                caps.scroll_axes.vertical
            };
            if !supports_axis {
                continue;
            }

            let can_scroll = if dominant_horizontal {
                caps.scroll_edges.can_scroll_horizontal(step_dx)
            } else {
                caps.scroll_edges.can_scroll_vertical(step_dy)
            };

            if can_scroll {
                chosen_winner = Some(surface_id);
                break;
            }

            // At boundary: either defer to parent or consume locally (overscroll bounce).
            if caps.nested_scroll_enabled {
                if idx == 0 {
                    deferred_inner = true;
                }
                continue;
            } else {
                chosen_winner = Some(surface_id);
                consumed_boundary_locally = true;
                break;
            }
        }

        let mut cancelled_surfaces = Vec::new();
        if let Some(prev) = prev_winner
            && Some(prev) != chosen_winner
        {
            cancelled_surfaces.push(prev);
        }

        if let Some(track) = track_opt {
            track.winner = chosen_winner;
            track.locked_scroll = chosen_winner.is_some();
        }

        let (innermost_disposition, reason) = match chosen_winner {
            Some(w) if w == innermost => {
                if consumed_boundary_locally {
                    (
                        GestureDisposition::Claim,
                        ArbitrationReason::ScrollBoundaryConsumedLocally,
                    )
                } else if prev_locked_scroll && prev_winner == Some(innermost) {
                    (
                        GestureDisposition::Claim,
                        ArbitrationReason::LockedWinnerContinued,
                    )
                } else {
                    (
                        GestureDisposition::Claim,
                        ArbitrationReason::ScrollAxisClaimed,
                    )
                }
            }
            Some(_) => {
                if deferred_inner {
                    (
                        GestureDisposition::DeferToParent,
                        ArbitrationReason::ScrollBoundaryDeferredToParent,
                    )
                } else {
                    (
                        GestureDisposition::PassThrough,
                        ArbitrationReason::OrthogonalAxisPassThrough,
                    )
                }
            }
            None => {
                if deferred_inner {
                    (
                        GestureDisposition::DeferToParent,
                        ArbitrationReason::ScrollBoundaryDeferredToParent,
                    )
                } else {
                    (
                        GestureDisposition::PassThrough,
                        ArbitrationReason::NoEligibleSurface,
                    )
                }
            }
        };

        ArbitrationDecision {
            innermost_disposition,
            winner: chosen_winner,
            cancelled_surfaces,
            reason,
        }
    }

    #[inline]
    fn handle_end(&mut self, innermost: SurfaceId, sample: PointerSample) -> ArbitrationDecision {
        let removed = self.active_pointers.remove(sample.pointer_id);
        let winner = removed.as_ref().and_then(|t| t.winner);
        let cancelled = if sample.phase == PointerPhase::Cancel {
            winner.into_iter().collect()
        } else {
            Vec::new()
        };
        let innermost_disposition = if winner == Some(innermost) {
            GestureDisposition::Claim
        } else {
            GestureDisposition::PassThrough
        };
        ArbitrationDecision {
            innermost_disposition,
            winner,
            cancelled_surfaces: cancelled,
            reason: ArbitrationReason::PointerEnded,
        }
    }

    #[inline(always)]
    fn is_interactive(&self, surface_id: SurfaceId) -> bool {
        self.capabilities
            .get(surface_id)
            .map(|c| c.handles_tap || c.scroll_axes.any())
            .unwrap_or(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn surface_chain_inline_and_heap_spill_preserve_order_and_retain() {
        let small: Vec<SurfaceId> = (1..=INLINE_CHAIN_CAP as u32).collect();
        let mut inline_chain = SurfaceChain::from_slice(&small);
        assert!(matches!(inline_chain, SurfaceChain::Inline { .. }));
        assert_eq!(inline_chain.as_slice(), small.as_slice());
        inline_chain.retain(|id| id % 2 == 0);
        let expected_small: Vec<SurfaceId> = small.into_iter().filter(|id| id % 2 == 0).collect();
        assert_eq!(inline_chain.as_slice(), expected_small.as_slice());

        let deep: Vec<SurfaceId> = (1..=(INLINE_CHAIN_CAP as u32 + 5)).collect();
        let mut heap_chain = SurfaceChain::from_slice(&deep);
        assert!(matches!(heap_chain, SurfaceChain::Heap(_)));
        assert_eq!(heap_chain.as_slice(), deep.as_slice());
        heap_chain.retain(|id| id > 10);
        let expected_deep: Vec<SurfaceId> = deep.into_iter().filter(|&id| id > 10).collect();
        assert_eq!(heap_chain.as_slice(), expected_deep.as_slice());
    }

    #[test]
    fn fast_u32_map_inline_spill_remove_and_retain() {
        let mut map: FastU32Map<u32, 4> = FastU32Map::default();
        assert!(map.is_empty());

        for k in 1..=4u32 {
            assert_eq!(map.insert(k, k * 10), None);
        }
        assert!(matches!(map, FastU32Map::Inline { .. }));
        assert_eq!(map.insert(2, 200), Some(20));
        assert_eq!(map.get(2), Some(&200));
        assert_eq!(map.remove(3), Some(30));
        assert_eq!(map.len(), 3);
        assert_eq!(map.get(3), None);

        // Refill and trigger spill to FxHashMap.
        assert_eq!(map.insert(3, 30), None);
        assert_eq!(map.insert(5, 50), None);
        assert!(matches!(map, FastU32Map::Spill(_)));
        assert_eq!(map.len(), 5);
        assert_eq!(map.get(1), Some(&10));
        assert_eq!(map.get(2), Some(&200));
        assert_eq!(map.get(5), Some(&50));

        map.retain(|k, v| {
            *v += 1;
            k % 2 == 1
        });
        assert_eq!(map.len(), 3);
        assert_eq!(map.get(1), Some(&11));
        assert_eq!(map.get(3), Some(&31));
        assert_eq!(map.get(5), Some(&51));
        assert_eq!(map.get(2), None);
    }

    #[test]
    fn gesture_arbitrator_handles_deep_chains_and_many_surfaces_across_spill() {
        let mut arb = GestureArbitrator::new();
        // Register 24 surfaces (> inline cap 16) and 12 active pointers (> inline cap 8).
        let chain: Vec<SurfaceId> = (1..=24u32).collect();
        for &sid in &chain {
            let mut caps = GestureCapabilities::vertical_scroll(FrameworkKind::Nve);
            if sid < 20 {
                caps.scroll_edges.at_bottom = true;
            }
            arb.set_capabilities(sid, caps);
        }

        for ptr in 1..=12u32 {
            let down = arb.arbitrate(
                &chain,
                PointerSample {
                    pointer_id: ptr,
                    phase: PointerPhase::Down,
                    x: 10.0,
                    y: 100.0,
                    delta_x: 0.0,
                    delta_y: 0.0,
                    timestamp_us: u64::from(ptr) * 1000,
                },
            );
            assert_eq!(down.winner, Some(1));

            let mv = arb.arbitrate(
                &chain,
                PointerSample {
                    pointer_id: ptr,
                    phase: PointerPhase::Move,
                    x: 10.0,
                    y: 60.0,
                    delta_x: 0.0,
                    delta_y: -40.0,
                    timestamp_us: u64::from(ptr) * 1000 + 500,
                },
            );
            assert_eq!(mv.winner, Some(20));
            assert_eq!(mv.innermost_disposition, GestureDisposition::DeferToParent);
            assert_eq!(mv.cancelled_surfaces, vec![1]);
        }
        assert_eq!(arb.active_pointer_count(), 12);

        // Unregistering winner clears it from every active pointer.
        arb.unregister_surface(20);
        for ptr in 1..=12u32 {
            let end = arb.arbitrate(
                &chain,
                PointerSample {
                    pointer_id: ptr,
                    phase: PointerPhase::Up,
                    x: 10.0,
                    y: 60.0,
                    delta_x: 0.0,
                    delta_y: 0.0,
                    timestamp_us: 99_000,
                },
            );
            assert_eq!(end.winner, None);
            assert_eq!(end.reason, ArbitrationReason::PointerEnded);
        }
        assert_eq!(arb.active_pointer_count(), 0);
    }
}
