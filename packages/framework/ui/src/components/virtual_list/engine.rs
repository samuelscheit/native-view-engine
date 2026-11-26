//! Pure virtualized list geometry (`HeightsRevision` and `RowHeights`).
//!
//! Supports 100,000+ uniform or variable-height rows with:
//! - Sealed invariants: both uniform and variable tables store exact integer `u32 >= 1` row heights
//!   and exact `u64` offsets/totals/spacers, making uniform↔variable promotion completely lossless.
//! - Strictly **O(1)** `Clone`, `PartialEq`, `Eq`, and revision checks on [`RowHeights`] with zero
//!   hashing and zero per-render O(N) deep scans or clones.
//! - At most **2 O(log N)** bit-lifting walks per variable-height [`RowHeights::slice`] when
//!   `overscan <= 32` (stepping small overscan windows via sequential `heights` reads), or up to
//!   **2** bit-lifts plus **2** `prefix_sum` walks when `overscan > 32`, with exact `u64` pixel
//!   conservation (`top_spacer_px + rendered_height_px + bottom_spacer_px == total_height_px`).

use std::mem::size_of;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use super::fenwick::FenwickHeightTree;
use super::state::VirtualSlice;

/// Overscan window threshold below which spacer offsets are stepped via sequential O(1) `heights`
/// reads instead of an O(log N) Fenwick `prefix_sum` walk.
const SMALL_OVERSCAN_LINEAR_THRESHOLD: usize = 32;

static NEXT_HEIGHTS_REVISION: AtomicU64 = AtomicU64::new(1);

#[inline]
fn next_variable_revision_id() -> u64 {
    NEXT_HEIGHTS_REVISION.fetch_add(1, Ordering::Relaxed)
}

/// Sanitizes a viewport or extent value: non-finite or `<= 0.0` maps to `0.0`.
#[inline]
pub(super) fn sanitize_extent(value: f64) -> f64 {
    if value.is_finite() && value > 0.0 {
        value
    } else {
        0.0
    }
}

/// Clamps a raw scroll offset to `[0.0, max(0.0, total_height - sanitize_extent(viewport_height))]`.
#[inline]
pub(super) fn clamp_scroll(raw_offset: f64, total_height: u64, viewport_height: f64) -> f64 {
    let vp = sanitize_extent(viewport_height);
    let max_scroll = ((total_height as f64) - vp).max(0.0);
    if !raw_offset.is_finite() || raw_offset <= 0.0 {
        0.0
    } else {
        raw_offset.min(max_scroll)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum RevisionInner {
    Uniform { count: usize, row_height: u32 },
    Variable(u64),
}

/// Opaque O(1) revision token identifying a specific [`RowHeights`] dataset or mutation version.
///
/// Two [`RowHeights`] values share a revision iff they have identical uniform `(count, row_height)`
/// parameters or refer to the same unmutated variable-height table revision. Comparing revisions
/// is strictly **O(1)** with zero hashing and zero collisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HeightsRevision(RevisionInner);

#[derive(Debug, Clone)]
enum RowHeightsInner {
    Uniform {
        count: usize,
        row_height: u32,
    },
    Variable {
        tree: Arc<FenwickHeightTree>,
        revision: u64,
    },
}

/// Sealed row height model supporting both zero-allocation uniform lists and variable-height
/// Fenwick lists.
///
/// Both uniform and variable representations use exact integer `u32 >= 1` pixel heights and exact
/// `u64` totals/offsets so uniform→variable promotion changes zero row heights by zero pixels.
///
/// `Clone`, `PartialEq`, and `Eq` are strictly **O(1)** with zero heap allocations: variable tables
/// wrap an `Arc<FenwickHeightTree>` paired with a monotonic revision so props diffing and
/// component re-renders never clone or scan the underlying `O(N)` table. Use
/// [`RowHeights::content_eq`] when an explicit value-level comparison is required.
#[derive(Debug, Clone)]
pub struct RowHeights(RowHeightsInner);

impl PartialEq for RowHeights {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (
                RowHeightsInner::Uniform {
                    count: c1,
                    row_height: h1,
                },
                RowHeightsInner::Uniform {
                    count: c2,
                    row_height: h2,
                },
            ) => c1 == c2 && h1 == h2,
            (
                RowHeightsInner::Variable {
                    tree: t1,
                    revision: r1,
                },
                RowHeightsInner::Variable {
                    tree: t2,
                    revision: r2,
                },
            ) => r1 == r2 && Arc::ptr_eq(t1, t2),
            _ => false,
        }
    }
}

impl Eq for RowHeights {}

impl RowHeights {
    /// Creates a zero-allocation O(1) uniform-height table (`row_height < 1` is clamped to `1`).
    #[inline]
    pub fn uniform(count: usize, row_height: u32) -> Self {
        Self(RowHeightsInner::Uniform {
            count,
            row_height: row_height.max(1),
        })
    }

    /// Creates a variable-height table backed by [`FenwickHeightTree`], normalizing `heights` in
    /// place in O(N) time (`< 1` clamped to `1`).
    pub fn variable(heights: Vec<u32>) -> Self {
        Self::from_tree(FenwickHeightTree::from_heights(heights))
    }

    /// Constructs a variable-height table in a single **O(N)** pass without an intermediate `Vec`.
    pub fn variable_from_fn(len: usize, height_at: impl FnMut(usize) -> u32) -> Self {
        Self::from_tree(FenwickHeightTree::from_fn(len, height_at))
    }

    /// Wraps an existing [`FenwickHeightTree`] with a fresh monotonic revision.
    pub fn from_tree(tree: FenwickHeightTree) -> Self {
        Self(RowHeightsInner::Variable {
            tree: Arc::new(tree),
            revision: next_variable_revision_id(),
        })
    }

    #[inline]
    pub fn is_variable(&self) -> bool {
        matches!(self.0, RowHeightsInner::Variable { .. })
    }

    /// Returns an exact, collision-free O(1) revision token for change detection.
    #[inline]
    pub fn revision(&self) -> HeightsRevision {
        match &self.0 {
            RowHeightsInner::Uniform { count, row_height } => {
                HeightsRevision(RevisionInner::Uniform {
                    count: *count,
                    row_height: *row_height,
                })
            }
            RowHeightsInner::Variable { revision, .. } => {
                HeightsRevision(RevisionInner::Variable(*revision))
            }
        }
    }

    /// Explicit value-level equality check (may run in **O(N)** for independently constructed
    /// variable tables or uniform-vs-variable comparisons).
    pub fn content_eq(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (
                RowHeightsInner::Uniform {
                    count: c1,
                    row_height: h1,
                },
                RowHeightsInner::Uniform {
                    count: c2,
                    row_height: h2,
                },
            ) => c1 == c2 && h1 == h2,
            (
                RowHeightsInner::Variable { tree: t1, .. },
                RowHeightsInner::Variable { tree: t2, .. },
            ) => Arc::ptr_eq(t1, t2) || t1 == t2,
            (
                RowHeightsInner::Uniform { count, row_height },
                RowHeightsInner::Variable { tree, .. },
            )
            | (
                RowHeightsInner::Variable { tree, .. },
                RowHeightsInner::Uniform { count, row_height },
            ) => *count == tree.len() && tree.heights_slice().iter().all(|&h| h == *row_height),
        }
    }

    #[inline]
    pub fn as_tree(&self) -> Option<&FenwickHeightTree> {
        match &self.0 {
            RowHeightsInner::Uniform { .. } => None,
            RowHeightsInner::Variable { tree, .. } => Some(tree),
        }
    }

    /// Approximate total memory footprint (stack struct + shared Fenwick buffers) in bytes.
    #[inline]
    pub fn memory_bytes(&self) -> usize {
        match &self.0 {
            RowHeightsInner::Uniform { .. } => size_of::<Self>(),
            RowHeightsInner::Variable { tree, .. } => size_of::<Self>() + tree.memory_bytes(),
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        match &self.0 {
            RowHeightsInner::Uniform { count, .. } => *count,
            RowHeightsInner::Variable { tree, .. } => tree.len(),
        }
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Exact total document height in logical pixels (`u64`, overflow-proof across millions of
    /// rows) in **O(1)** time.
    #[inline]
    pub fn total_height(&self) -> u64 {
        match &self.0 {
            RowHeightsInner::Uniform { count, row_height } => {
                let total = (*count as u64).checked_mul(u64::from(*row_height));
                debug_assert!(
                    total.is_some(),
                    "RowHeights::Uniform total height overflowed u64::MAX"
                );
                total.unwrap_or(u64::MAX)
            }
            RowHeightsInner::Variable { tree, .. } => tree.total_height(),
        }
    }

    /// Exact height of row `index` in logical pixels (`0` if out of bounds) in **O(1)** time.
    #[inline]
    pub fn row_height(&self, index: usize) -> u32 {
        match &self.0 {
            RowHeightsInner::Uniform { count, row_height } => {
                if index < *count {
                    *row_height
                } else {
                    0
                }
            }
            RowHeightsInner::Variable { tree, .. } => tree.height_at(index).unwrap_or(0),
        }
    }

    /// Exact Y pixel offset of row `index` as `u64` (**O(1)** uniform, **O(log N)** variable).
    #[inline]
    pub fn offset_of(&self, index: usize) -> u64 {
        match &self.0 {
            RowHeightsInner::Uniform { count, row_height } => {
                (index.min(*count) as u64).saturating_mul(u64::from(*row_height))
            }
            RowHeightsInner::Variable { tree, .. } => tree.prefix_sum(index),
        }
    }

    /// Updates a single row's height (`new_height < 1` is clamped to `1`).
    ///
    /// Returns `false` if `index` is out of bounds (`index >= self.len()`), and `true` otherwise.
    ///
    /// # Complexity & Uniform→Variable Promotion
    ///
    /// - In `Uniform` mode: out-of-bounds calls and no-op calls (`new_height.max(1) == row_height`)
    ///   return in **O(1)** with zero allocations and keep `self` in `Uniform` mode. The **first
    ///   heterogeneous height change** performs a **one-time O(N)** lossless promotion to
    ///   `Variable` (`FenwickHeightTree`), after which all subsequent updates run in **O(log N)**
    ///   with zero heap allocations when uniquely owned.
    /// - In `Variable` mode: unchanged heights return in **O(1)** without cloning the `Arc` or
    ///   bumping [`Self::revision`]. Mutating updates run in **O(log N)** via `Arc::make_mut`
    ///   (zero heap allocations when uniquely owned; copy-on-write when shared) and bump
    ///   [`Self::revision`].
    pub fn set_row_height(&mut self, index: usize, new_height: u32) -> bool {
        let clamped = new_height.max(1);
        match &mut self.0 {
            RowHeightsInner::Uniform { count, row_height } => {
                if index >= *count {
                    return false;
                }
                if clamped == *row_height {
                    return true;
                }
                let base_h = *row_height;
                let tree = FenwickHeightTree::from_fn(*count, |i| {
                    if i == index { clamped } else { base_h }
                });
                self.0 = RowHeightsInner::Variable {
                    tree: Arc::new(tree),
                    revision: next_variable_revision_id(),
                };
                true
            }
            RowHeightsInner::Variable { tree, revision } => {
                let Some(old_h) = tree.height_at(index) else {
                    return false;
                };
                if old_h == clamped {
                    return true;
                }
                Arc::make_mut(tree).set_height(index, clamped);
                *revision = next_variable_revision_id();
                true
            }
        }
    }

    /// Returns `(index, row_top_px)` for the row covering `offset_px` (or `(len(), total_height())`
    /// when `offset_px >= total_height()`).
    ///
    /// Runs in **O(1)** for `Uniform` and a single **O(log N)** bit-lift for `Variable`.
    pub fn index_and_offset_at(&self, offset_px: f64) -> (usize, u64) {
        let len = self.len();
        if len == 0 || !offset_px.is_finite() || offset_px <= 0.0 {
            return (0, 0);
        }
        let target_u64 = offset_px.floor() as u64;
        match &self.0 {
            RowHeightsInner::Uniform { count, row_height } => {
                let rh = u64::from(*row_height);
                let idx = (target_u64 / rh).min(*count as u64) as usize;
                (idx, (idx as u64).saturating_mul(rh))
            }
            RowHeightsInner::Variable { tree, .. } => tree.lower_bound_with_offset(target_u64),
        }
    }

    #[inline]
    pub fn index_at_offset(&self, offset_px: f64) -> usize {
        self.index_and_offset_at(offset_px).0
    }

    /// Computes the O(viewport) visible + overscan slice with exact `u64` top/rendered/bottom pixel
    /// conservation (`top_spacer_px + rendered_height_px + bottom_spacer_px == total_height_px`).
    ///
    /// Runs in **O(1)** for `Uniform`. For `Variable`, performs at most **2 O(log N)** bit-lifting
    /// walks when `overscan <= 32` (stepping small overscan windows via sequential `heights`
    /// reads), or up to **2** bit-lifts plus **2** `prefix_sum` walks when `overscan > 32`.
    pub fn slice(&self, scroll_offset: f64, viewport_height: f64, overscan: usize) -> VirtualSlice {
        let len = self.len();
        if len == 0 {
            return VirtualSlice::EMPTY;
        }

        let vp = sanitize_extent(viewport_height);
        let total_u64 = self.total_height();
        let offset = clamp_scroll(scroll_offset, total_u64, vp);
        let offset_u64 = offset.floor() as u64;
        let end_px_f64 = offset + vp;
        let end_px_u64 = end_px_f64.floor() as u64;

        match &self.0 {
            RowHeightsInner::Uniform { count, row_height } => {
                let rh_u64 = u64::from(*row_height);
                let first_visible = (offset_u64 / rh_u64).min((*count - 1) as u64) as usize;
                let end_visible = if end_px_u64 >= total_u64 {
                    *count
                } else {
                    let raw_end = (end_px_u64 / rh_u64).min(*count as u64) as usize;
                    let row_top = (raw_end as u64).saturating_mul(rh_u64);
                    if row_top < end_px_u64
                        || end_px_f64 > (end_px_u64 as f64)
                        || raw_end == first_visible
                    {
                        (raw_end + 1).min(*count)
                    } else {
                        raw_end
                    }
                };

                let start = first_visible.saturating_sub(overscan);
                let end = end_visible.saturating_add(overscan).min(*count);
                let top_spacer_px = (start as u64).saturating_mul(rh_u64);
                let end_offset_px = (end as u64).saturating_mul(rh_u64);
                let rendered_height_px = end_offset_px.saturating_sub(top_spacer_px);
                let bottom_spacer_px = total_u64.saturating_sub(end_offset_px);

                VirtualSlice {
                    start,
                    end,
                    first_visible,
                    end_visible,
                    clamped_scroll_offset: offset,
                    top_spacer_px,
                    rendered_height_px,
                    bottom_spacer_px,
                    total_height_px: total_u64,
                    visible_count: end.saturating_sub(start),
                }
            }
            RowHeightsInner::Variable { tree, .. } => {
                let heights = tree.heights_slice();
                let (raw_first, raw_first_top) = tree.lower_bound_with_offset(offset_u64);
                let (first_visible, first_top_u64) = if raw_first < len {
                    (raw_first, raw_first_top)
                } else {
                    let last = len - 1;
                    (last, total_u64.saturating_sub(u64::from(heights[last])))
                };

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

                let top_spacer_px = if start == 0 {
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

                let end_offset_px = if end == len {
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

                let rendered_height_px = end_offset_px.saturating_sub(top_spacer_px);
                let bottom_spacer_px = total_u64.saturating_sub(end_offset_px);

                VirtualSlice {
                    start,
                    end,
                    first_visible,
                    end_visible,
                    clamped_scroll_offset: offset,
                    top_spacer_px,
                    rendered_height_px,
                    bottom_spacer_px,
                    total_height_px: total_u64,
                    visible_count: end.saturating_sub(start),
                }
            }
        }
    }
}
