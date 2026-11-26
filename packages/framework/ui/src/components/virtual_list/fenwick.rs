//! Integer-pixel Fenwick tree (Binary Indexed Tree) backing variable-height virtual lists.

use std::mem::size_of;

#[inline]
pub(super) fn sanitize_height_f32_to_u32(h: f32) -> u32 {
    if h.is_finite() && h >= 1.0 {
        h.round().min(u32::MAX as f32) as u32
    } else {
        1
    }
}

/// Reusable integer-pixel Fenwick tree (Binary Indexed Tree) storing per-row heights with
/// O(N) construction, O(1) total height, and O(log N) prefix sums, height updates, and
/// bit-lifting `lower_bound_with_offset` queries.
///
/// # Memory Footprint
///
/// Stores `heights: Vec<u32>` (4 B/row, supporting tall rows up to `u32::MAX` px) and
/// `bit: Vec<u64>` (8 B/row, overflow-proof `u64` prefix sums across millions of rows), for an
/// exact heap footprint of **12 bytes/row** (+ 8 B sentinel `bit[0]` + `size_of::<Self>()`).
/// At 100,000 rows this is `1,200,008` bytes (~1.14 MiB); an empty tree allocates `0` bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FenwickHeightTree {
    heights: Vec<u32>,
    /// 1-indexed Binary Indexed Tree of `u64` prefix sums (overflow-proof across millions of rows).
    bit: Vec<u64>,
    total: u64,
}

impl FenwickHeightTree {
    /// Builds a Fenwick tree in strictly **O(N)** time, normalizing `heights` in place with zero
    /// intermediate height `Vec`. Row heights `< 1` are clamped to `1`.
    pub fn from_heights(mut heights: Vec<u32>) -> Self {
        let n = heights.len();
        if n == 0 {
            return Self {
                heights,
                bit: Vec::new(),
                total: 0,
            };
        }
        let mut bit = vec![0u64; n + 1];
        let mut total = 0u64;

        for (idx, slot) in heights.iter_mut().enumerate() {
            let h = (*slot).max(1);
            *slot = h;
            let h64 = u64::from(h);
            total = total.saturating_add(h64);
            let i = idx + 1;
            bit[i] = bit[i].saturating_add(h64);
            let parent = i + i.isolate_lowest_one();
            if parent <= n {
                bit[parent] = bit[parent].saturating_add(bit[i]);
            }
        }

        Self {
            heights,
            bit,
            total,
        }
    }

    /// Builds a Fenwick tree in a single **O(N)** pass without allocating an intermediate height
    /// `Vec`. Row heights `< 1` are clamped to `1`.
    pub fn from_fn(len: usize, mut height_at: impl FnMut(usize) -> u32) -> Self {
        if len == 0 {
            return Self {
                heights: Vec::new(),
                bit: Vec::new(),
                total: 0,
            };
        }
        let mut heights = Vec::with_capacity(len);
        let mut bit = vec![0u64; len + 1];
        let mut total = 0u64;

        for idx in 0..len {
            let h = height_at(idx).max(1);
            heights.push(h);
            let h64 = u64::from(h);
            total = total.saturating_add(h64);
            let i = idx + 1;
            bit[i] = bit[i].saturating_add(h64);
            let parent = i + i.isolate_lowest_one();
            if parent <= len {
                bit[parent] = bit[parent].saturating_add(bit[i]);
            }
        }

        Self {
            heights,
            bit,
            total,
        }
    }

    /// Builds a Fenwick tree from floating-point row heights in one pass, sanitizing
    /// NaN/negative/zero values to `1`.
    pub fn from_f32_heights(heights: &[f32]) -> Self {
        Self::from_fn(heights.len(), |idx| sanitize_height_f32_to_u32(heights[idx]))
    }

    /// Builds a uniform-height Fenwick tree of `len` rows in **O(N)** time.
    pub fn uniform(len: usize, row_height: u32) -> Self {
        let h = row_height.max(1);
        Self::from_fn(len, |_| h)
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.heights.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.heights.is_empty()
    }

    /// Total document height in logical pixels in **O(1)** time.
    #[inline]
    pub fn total_height(&self) -> u64 {
        self.total
    }

    /// Height of a single row in **O(1)** time.
    #[inline]
    pub fn height_at(&self, index: usize) -> Option<u32> {
        self.heights.get(index).copied()
    }

    #[inline]
    pub(super) fn heights_slice(&self) -> &[u32] {
        &self.heights
    }

    /// Approximate total memory footprint (stack struct + heap buffers) in bytes.
    #[inline]
    pub fn memory_bytes(&self) -> usize {
        size_of::<Self>()
            + self.heights.capacity() * size_of::<u32>()
            + self.bit.capacity() * size_of::<u64>()
    }

    /// Updates the height of `index` in **O(log N)** time.
    ///
    /// Returns `false` if `index` is out of bounds (`index >= self.len()`), and `true` otherwise.
    /// Updating with the existing clamped height is an O(1) no-op.
    pub fn set_height(&mut self, index: usize, new_height: u32) -> bool {
        let Some(&old_h) = self.heights.get(index) else {
            return false;
        };
        let clamped = new_height.max(1);
        if clamped == old_h {
            return true;
        }
        self.heights[index] = clamped;
        let old64 = u64::from(old_h);
        let new64 = u64::from(clamped);
        let n = self.heights.len();
        let mut i = index + 1;
        if new64 > old64 {
            let diff = new64 - old64;
            self.total = self.total.saturating_add(diff);
            while i <= n {
                self.bit[i] = self.bit[i].saturating_add(diff);
                i += i.isolate_lowest_one();
            }
        } else {
            let diff = old64 - new64;
            self.total = self.total.saturating_sub(diff);
            while i <= n {
                self.bit[i] = self.bit[i].saturating_sub(diff);
                i += i.isolate_lowest_one();
            }
        }
        true
    }

    /// Cumulative pixel height of rows `[0..count)` in **O(log N)** time.
    pub fn prefix_sum(&self, count: usize) -> u64 {
        let mut i = count.min(self.heights.len());
        let mut sum = 0u64;
        while i > 0 {
            sum = sum.saturating_add(self.bit[i]);
            i -= i.isolate_lowest_one();
        }
        sum
    }

    /// Bit-lifting **O(log N)** lower bound returning `(index, row_top_px)`:
    /// - `index` is the smallest row index `r` whose bottom edge strictly exceeds `offset_px`, or
    ///   `len()` when `offset_px >= total_height()`.
    /// - `row_top_px` is the exact cumulative `prefix_sum(index)` accumulated during the descent,
    ///   eliminating any follow-up `prefix_sum` walk.
    pub fn lower_bound_with_offset(&self, offset_px: u64) -> (usize, u64) {
        let n = self.heights.len();
        if n == 0 || offset_px == 0 {
            return (0, 0);
        }
        if offset_px >= self.total {
            return (n, self.total);
        }
        let mut idx = 0usize;
        let mut acc = 0u64;
        let mut step = 1usize << (usize::BITS - 1 - n.leading_zeros());
        while step > 0 {
            let next = idx + step;
            if next <= n {
                let candidate = acc.saturating_add(self.bit[next]);
                if candidate <= offset_px {
                    idx = next;
                    acc = candidate;
                }
            }
            step >>= 1;
        }
        (idx, acc)
    }

    /// Bit-lifting **O(log N)** lower bound: returns the smallest row index `r` whose bottom edge
    /// strictly exceeds `offset_px`, or `len()` when `offset_px >= total_height()`.
    #[inline]
    pub fn lower_bound(&self, offset_px: u64) -> usize {
        self.lower_bound_with_offset(offset_px).0
    }
}
