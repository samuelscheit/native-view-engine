//! Pure virtualized list geometry (`HeightsRevision` and `RowHeights`).
//!
//! Supports 100,000+ uniform or variable-height rows with:
//! - Strictly **O(1)** `Clone`, `PartialEq`, and revision checks on [`RowHeights`] (zero per-render
//!   O(N) deep tree compares or clones).
//! - At most **2 O(log N)** bit-lifting walks per variable-height [`RowHeights::slice`] (down from
//!   5) with exact `u64` pixel accounting (`top_spacer_px + rendered_height_px + bottom_spacer_px
//!   == total_height_px`).

use std::mem::size_of;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use super::fenwick::{sanitize_height_f32_to_u32, FenwickHeightTree};
use super::state::VirtualSlice;

/// Overscan window threshold below which spacer offsets are stepped via sequential O(1) `heights`
/// reads instead of an O(log N) Fenwick `prefix_sum` walk.
const SMALL_OVERSCAN_LINEAR_THRESHOLD: usize = 32;

/// High bit reserved for deterministic [`RowHeights::Uniform`] revisions so monotonic
/// [`RowHeights::Variable`] revisions can never collide with them.
const UNIFORM_REVISION_TAG: u64 = 1u64 << 63;

static NEXT_HEIGHTS_REVISION: AtomicU64 = AtomicU64::new(1);

#[inline]
fn next_variable_revision() -> HeightsRevision {
    let raw = NEXT_HEIGHTS_REVISION.fetch_add(1, Ordering::Relaxed) & !UNIFORM_REVISION_TAG;
    HeightsRevision(raw.max(1))
}

#[inline]
fn uniform_revision(count: usize, row_height: f32) -> HeightsRevision {
    let mut buf = [0u8; 12];
    buf[..8].copy_from_slice(&(count as u64).to_le_bytes());
    buf[8..12].copy_from_slice(&row_height.to_bits().to_le_bytes());
    HeightsRevision(nve_engine::xxh64(&buf, 0) | UNIFORM_REVISION_TAG)
}

/// Typed O(1) revision token identifying a specific [`RowHeights`] dataset or mutation version.
///
/// Every newly created variable height table receives a monotonically increasing revision, and
/// every mutating [`RowHeights::set_row_height`] call bumps it, enabling O(1) change detection
/// without scanning `O(N)` row heights on every render frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(transparent)]
pub struct HeightsRevision(u64);

impl HeightsRevision {
    #[inline]
    pub const fn raw(self) -> u64 {
        self.0
    }
}

/// Row height model supporting both zero-allocation uniform lists and variable-height Fenwick lists.
///
/// `Clone` and `PartialEq` are strictly **O(1)** with zero heap allocations: `Variable` wraps an
/// `Arc<FenwickHeightTree>` paired with a monotonic [`HeightsRevision`] so props diffing and
/// component re-renders never clone or scan the underlying `O(N)` table. Use
/// [`RowHeights::content_eq`] when an explicit value-level comparison is required.
#[derive(Debug, Clone)]
pub enum RowHeights {
    /// Constant row height in logical pixels — strictly O(1) offset/slice math, zero allocation.
    Uniform { count: usize, row_height: f32 },
    /// Variable per-row heights backed by a shared [`FenwickHeightTree`] and monotonic
    /// [`HeightsRevision`] — O(1) `Clone`/`PartialEq`, O(log N) prefix sums & lookups, and
    /// zero-allocation O(log N) in-place updates via `Arc::make_mut` when uniquely owned.
    Variable {
        tree: Arc<FenwickHeightTree>,
        revision: HeightsRevision,
    },
}

impl PartialEq for RowHeights {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (
                Self::Uniform {
                    count: c1,
                    row_height: h1,
                },
                Self::Uniform {
                    count: c2,
                    row_height: h2,
                },
            ) => c1 == c2 && h1.to_bits() == h2.to_bits(),
            (
                Self::Variable {
                    tree: t1,
                    revision: r1,
                },
                Self::Variable {
                    tree: t2,
                    revision: r2,
                },
            ) => r1 == r2 && Arc::ptr_eq(t1, t2),
            _ => false,
        }
    }
}

impl RowHeights {
    pub fn uniform(count: usize, row_height: f32) -> Self {
        let h = if row_height.is_finite() && row_height >= 1.0 {
            row_height
        } else {
            1.0
        };
        Self::Uniform {
            count,
            row_height: h,
        }
    }

    pub fn variable(heights: Vec<u32>) -> Self {
        Self::from_tree(FenwickHeightTree::from_heights(heights))
    }

    /// Constructs a variable-height table in a single **O(N)** pass without an intermediate `Vec`.
    pub fn variable_from_fn(len: usize, height_at: impl FnMut(usize) -> u32) -> Self {
        Self::from_tree(FenwickHeightTree::from_fn(len, height_at))
    }

    /// Wraps an existing [`FenwickHeightTree`] with a fresh [`HeightsRevision`].
    pub fn from_tree(tree: FenwickHeightTree) -> Self {
        Self::Variable {
            tree: Arc::new(tree),
            revision: next_variable_revision(),
        }
    }

    #[inline]
    pub fn is_variable(&self) -> bool {
        matches!(self, Self::Variable { .. })
    }

    /// Returns an O(1) revision token for change detection.
    #[inline]
    pub fn revision(&self) -> HeightsRevision {
        match self {
            Self::Uniform { count, row_height } => uniform_revision(*count, *row_height),
            Self::Variable { revision, .. } => *revision,
        }
    }

    /// Explicit value-level equality check (may run in **O(N)** for independently constructed
    /// `Variable` tables or `Uniform` vs `Variable` comparisons).
    pub fn content_eq(&self, other: &Self) -> bool {
        match (self, other) {
            (
                Self::Uniform {
                    count: c1,
                    row_height: h1,
                },
                Self::Uniform {
                    count: c2,
                    row_height: h2,
                },
            ) => c1 == c2 && h1.to_bits() == h2.to_bits(),
            (Self::Variable { tree: t1, .. }, Self::Variable { tree: t2, .. }) => {
                Arc::ptr_eq(t1, t2) || t1 == t2
            }
            (Self::Uniform { count, row_height }, Self::Variable { tree, .. })
            | (Self::Variable { tree, .. }, Self::Uniform { count, row_height }) => {
                if *count != tree.len() || row_height.fract() != 0.0 {
                    return false;
                }
                let expected = sanitize_height_f32_to_u32(*row_height);
                tree.heights_slice().iter().all(|&h| h == expected)
            }
        }
    }

    #[inline]
    pub fn as_tree(&self) -> Option<&FenwickHeightTree> {
        match self {
            Self::Uniform { .. } => None,
            Self::Variable { tree, .. } => Some(tree),
        }
    }

    /// Approximate total memory footprint (stack enum + shared Fenwick buffers) in bytes.
    #[inline]
    pub fn memory_bytes(&self) -> usize {
        match self {
            Self::Uniform { .. } => size_of::<Self>(),
            Self::Variable { tree, .. } => size_of::<Self>() + tree.memory_bytes(),
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        match self {
            Self::Uniform { count, .. } => *count,
            Self::Variable { tree, .. } => tree.len(),
        }
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[inline]
    pub fn total_height(&self) -> f32 {
        match self {
            Self::Uniform { count, row_height } => (*count as f32) * *row_height,
            Self::Variable { tree, .. } => tree.total_height() as f32,
        }
    }

    /// Exact total height in integer logical pixels (`u64`, overflow-proof across millions of rows).
    #[inline]
    pub fn total_height_u64(&self) -> u64 {
        match self {
            Self::Uniform { count, row_height } => {
                ((*count as f64) * f64::from(*row_height)).round() as u64
            }
            Self::Variable { tree, .. } => tree.total_height(),
        }
    }

    #[inline]
    pub fn row_height(&self, index: usize) -> f32 {
        match self {
            Self::Uniform { count, row_height } => {
                if index < *count {
                    *row_height
                } else {
                    0.0
                }
            }
            Self::Variable { tree, .. } => tree.height_at(index).unwrap_or(0) as f32,
        }
    }

    #[inline]
    pub fn offset_of(&self, index: usize) -> f32 {
        match self {
            Self::Uniform { count, row_height } => (index.min(*count) as f32) * *row_height,
            Self::Variable { tree, .. } => tree.prefix_sum(index) as f32,
        }
    }

    /// Exact Y pixel offset of row `index` as `u64` (O(1) uniform, O(log N) variable).
    #[inline]
    pub fn offset_of_u64(&self, index: usize) -> u64 {
        match self {
            Self::Uniform { count, row_height } => {
                ((index.min(*count) as f64) * f64::from(*row_height)).round() as u64
            }
            Self::Variable { tree, .. } => tree.prefix_sum(index),
        }
    }

    /// Updates a single row's height (`new_height < 1` is clamped to `1`).
    ///
    /// Returns `false` if `index` is out of bounds (`index >= self.len()`), and `true` otherwise.
    ///
    /// # Complexity & Uniform→Variable Promotion
    ///
    /// - In `Uniform` mode: out-of-bounds calls and no-op calls (`new_height.max(1)` equal to an
    ///   integer `row_height`) return in **O(1)** with zero allocations and keep `self` in
    ///   `Uniform` mode. The **first heterogeneous height change** performs a **one-time O(N)**
    ///   promotion to `Variable` (`FenwickHeightTree`, rounding any fractional uniform height to
    ///   nearest `u32 >= 1` px), after which all subsequent updates run in **O(log N)** with zero
    ///   heap allocations when uniquely owned.
    /// - In `Variable` mode: unchanged heights return in **O(1)** without cloning the `Arc` or
    ///   bumping [`Self::revision`]. Mutating updates run in **O(log N)** via `Arc::make_mut`
    ///   (zero heap allocations when uniquely owned; copy-on-write when shared) and bump
    ///   [`Self::revision`].
    pub fn set_row_height(&mut self, index: usize, new_height: u32) -> bool {
        let clamped = new_height.max(1);
        match self {
            Self::Uniform { count, row_height } => {
                if index >= *count {
                    return false;
                }
                let base_h = sanitize_height_f32_to_u32(*row_height);
                if row_height.fract() == 0.0 && clamped == base_h {
                    return true;
                }
                let tree = FenwickHeightTree::from_fn(*count, |i| {
                    if i == index { clamped } else { base_h }
                });
                *self = Self::Variable {
                    tree: Arc::new(tree),
                    revision: next_variable_revision(),
                };
                true
            }
            Self::Variable { tree, revision } => {
                let Some(old_h) = tree.height_at(index) else {
                    return false;
                };
                if old_h == clamped {
                    return true;
                }
                Arc::make_mut(tree).set_height(index, clamped);
                *revision = next_variable_revision();
                true
            }
        }
    }

    /// Returns `(index, row_top_px)` for the row covering `offset_px` (or `(len(), total_height())`
    /// when `offset_px >= total_height()`).
    ///
    /// Runs in **O(1)** for `Uniform` and a single **O(log N)** bit-lift for `Variable`.
    pub fn index_and_offset_at(&self, offset_px: f32) -> (usize, f32) {
        let len = self.len();
        if len == 0 || !offset_px.is_finite() || offset_px <= 0.0 {
            return (0, 0.0);
        }
        match self {
            Self::Uniform { count, row_height } => {
                let idx = ((f64::from(offset_px) / f64::from(*row_height)).floor() as usize)
                    .min(*count);
                (idx, (idx as f32) * *row_height)
            }
            Self::Variable { tree, .. } => {
                let (idx, top_u64) = tree.lower_bound_with_offset(offset_px.floor() as u64);
                (idx, top_u64 as f32)
            }
        }
    }

    #[inline]
    pub fn index_at_offset(&self, offset_px: f32) -> usize {
        self.index_and_offset_at(offset_px).0
    }

    /// Computes the O(viewport) visible + overscan slice with exact top/rendered/bottom pixel
    /// accounting.
    ///
    /// Runs in **O(1)** for `Uniform` and at most **2 O(log N)** bit-lifting walks for `Variable`
    /// (stepping across small overscan windows `<= 32` via direct `heights` reads instead of extra
    /// Fenwick prefix-sum traversals).
    pub fn slice(&self, scroll_offset: f32, viewport_height: f32, overscan: usize) -> VirtualSlice {
        let len = self.len();
        if len == 0 {
            return VirtualSlice::EMPTY;
        }

        let vp = if viewport_height.is_finite() && viewport_height > 0.0 {
            viewport_height
        } else {
            0.0
        };

        match self {
            Self::Uniform { count, row_height } => {
                let total = (*count as f32) * *row_height;
                let max_scroll = (total - vp).max(0.0);
                let offset = if !scroll_offset.is_finite() || scroll_offset <= 0.0 {
                    0.0
                } else {
                    scroll_offset.min(max_scroll)
                };

                let row_h_f64 = f64::from(*row_height);
                let first_visible =
                    ((f64::from(offset) / row_h_f64).floor() as usize).min(*count - 1);
                let end_px = f64::from(offset) + f64::from(vp);
                let raw_end = ((end_px / row_h_f64).floor() as usize).min(*count);
                let end_visible = if raw_end < *count {
                    let row_top = (raw_end as f64) * row_h_f64;
                    if row_top < end_px || raw_end == first_visible {
                        (raw_end + 1).min(*count)
                    } else {
                        raw_end.max(first_visible + 1).min(*count)
                    }
                } else {
                    *count
                };

                let start = first_visible.saturating_sub(overscan);
                let end = end_visible.saturating_add(overscan).min(*count);
                let top_spacer_px = (start as f32) * *row_height;
                let end_offset_px = (end as f32) * *row_height;
                let rendered_height_px = (end_offset_px - top_spacer_px).max(0.0);
                let bottom_spacer_px = (total - end_offset_px).max(0.0);

                VirtualSlice {
                    start,
                    end,
                    first_visible,
                    end_visible,
                    clamped_scroll_offset: offset,
                    top_spacer_px,
                    rendered_height_px,
                    bottom_spacer_px,
                    total_height_px: total,
                    visible_count: end.saturating_sub(start),
                }
            }
            Self::Variable { tree, .. } => {
                let heights = tree.heights_slice();
                let total_u64 = tree.total_height();
                let total_f32 = total_u64 as f32;
                let max_scroll = (total_f32 - vp).max(0.0);
                let offset = if !scroll_offset.is_finite() || scroll_offset <= 0.0 {
                    0.0
                } else {
                    scroll_offset.min(max_scroll)
                };

                let offset_u64 = offset.floor() as u64;
                let (raw_first, raw_first_top) = tree.lower_bound_with_offset(offset_u64);
                let (first_visible, first_top_u64) = if raw_first < len {
                    (raw_first, raw_first_top)
                } else {
                    let last = len - 1;
                    (last, total_u64.saturating_sub(u64::from(heights[last])))
                };

                let end_px_f64 = f64::from(offset) + f64::from(vp);
                let end_px_u64 = end_px_f64.floor() as u64;
                let (raw_end, raw_end_top) = tree.lower_bound_with_offset(end_px_u64);
                let (end_visible, end_visible_top_u64) = if raw_end < len {
                    if raw_end_top < end_px_u64
                        || end_px_f64 > (end_px_u64 as f64)
                        || raw_end == first_visible
                    {
                        let next_top = raw_end_top.saturating_add(u64::from(heights[raw_end]));
                        (raw_end + 1, next_top)
                    } else {
                        (raw_end, raw_end_top)
                    }
                } else {
                    (len, total_u64)
                };

                let start = first_visible.saturating_sub(overscan);
                let end = end_visible.saturating_add(overscan).min(len);

                let top_spacer_u64 = if start == 0 {
                    0
                } else if first_visible - start <= SMALL_OVERSCAN_LINEAR_THRESHOLD {
                    let mut top = first_top_u64;
                    for &h in &heights[start..first_visible] {
                        top = top.saturating_sub(u64::from(h));
                    }
                    top
                } else {
                    tree.prefix_sum(start)
                };

                let end_offset_u64 = if end == len {
                    total_u64
                } else if end - end_visible <= SMALL_OVERSCAN_LINEAR_THRESHOLD {
                    let mut bottom = end_visible_top_u64;
                    for &h in &heights[end_visible..end] {
                        bottom = bottom.saturating_add(u64::from(h));
                    }
                    bottom
                } else {
                    tree.prefix_sum(end)
                };

                let rendered_u64 = end_offset_u64.saturating_sub(top_spacer_u64);
                let bottom_spacer_u64 = total_u64.saturating_sub(end_offset_u64);

                VirtualSlice {
                    start,
                    end,
                    first_visible,
                    end_visible,
                    clamped_scroll_offset: offset,
                    top_spacer_px: top_spacer_u64 as f32,
                    rendered_height_px: rendered_u64 as f32,
                    bottom_spacer_px: bottom_spacer_u64 as f32,
                    total_height_px: total_f32,
                    visible_count: end.saturating_sub(start),
                }
            }
        }
    }
}
