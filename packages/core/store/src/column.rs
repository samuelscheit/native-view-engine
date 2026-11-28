//! One table's storage: dense (row = node index) or paged-sparse (packed rows + owners).

use nve_abi::table::{NVE_ROW_INDEX_NO_PAGE, NVE_ROW_INDEX_PAGE_BITS, Storage, Word};

use crate::id::{NodeId, NodeIndex};
use crate::row::DirtyList;
use crate::table::TableMeta;
use crate::wire::WireError;
use crate::wire::budget::SessionBudget;

/// Published verbatim as `NveTableView::row_index_*`, so the page geometry is the ABI's.
const PAGE_BITS: u32 = NVE_ROW_INDEX_PAGE_BITS;
const PAGE: usize = 1 << PAGE_BITS;
const NO_PAGE: u32 = NVE_ROW_INDEX_NO_PAGE;

#[derive(Clone, Copy)]
struct PagedPlan {
    dir_target: usize,
    dir_delta: usize,
    pool_target: usize,
    pool_delta: usize,
}

impl PagedPlan {
    #[inline]
    fn total_delta(self) -> usize {
        self.dir_delta + self.pool_delta
    }
}

#[derive(Clone, Copy)]
struct RowBufferPlan {
    target_cap: usize,
    target_words: usize,
    masks_delta: usize,
    owners_delta: usize,
    rows_delta: usize,
}

/// Node index → packed row (`row + 1`, `0` = absent) in 256-entry pages; no per-node `Box`.
#[derive(Debug, Default)]
pub(crate) struct PagedIndex {
    dir: Vec<u32>,
    pool: Vec<[u32; PAGE]>,
}

impl PagedIndex {
    #[inline(always)]
    pub(crate) fn get(&self, node: NodeIndex) -> Option<u32> {
        let n = node.0;
        let page = *self.dir.get((n >> PAGE_BITS) as usize)?;
        if page == NO_PAGE {
            return None;
        }
        self.pool[page as usize][n as usize & (PAGE - 1)].checked_sub(1)
    }

    fn set(&mut self, node: NodeIndex, row_plus_one: u32) {
        let n = node.0;
        let p = (n >> PAGE_BITS) as usize;
        if self.dir.len() <= p {
            self.dir.resize(p + 1, NO_PAGE);
        }
        if self.dir[p] == NO_PAGE {
            self.dir[p] = self.pool.len() as u32;
            self.pool.push([0; PAGE]);
        }
        self.pool[self.dir[p] as usize][n as usize & (PAGE - 1)] = row_plus_one;
    }

    fn plan_try_set(
        &self,
        node: NodeIndex,
        remaining_bytes: usize,
    ) -> Result<PagedPlan, WireError> {
        let p = (node.0 >> PAGE_BITS) as usize;
        let needed_dir = p.checked_add(1).ok_or(WireError::Budget)?;
        let needs_page = self.dir.get(p).copied().unwrap_or(NO_PAGE) == NO_PAGE;
        let needed_pool = if needs_page {
            self.pool.len().checked_add(1).ok_or(WireError::Budget)?
        } else {
            self.pool.len()
        };

        let pool_old_cap = self.pool.capacity();
        let min_pool_delta = needed_pool
            .saturating_sub(pool_old_cap)
            .checked_mul(size_of::<[u32; PAGE]>())
            .ok_or(WireError::Budget)?;
        if min_pool_delta > remaining_bytes {
            return Err(WireError::Budget);
        }

        let dir_old_cap = self.dir.capacity();
        let (dir_target, dir_delta) = SessionBudget::bounded_target_cap(
            dir_old_cap,
            self.dir.len(),
            needed_dir,
            size_of::<u32>(),
            remaining_bytes - min_pool_delta,
        )?;
        let remaining_after_dir = remaining_bytes.saturating_sub(dir_delta);
        let (pool_target, pool_delta) = SessionBudget::bounded_target_cap(
            pool_old_cap,
            self.pool.len(),
            needed_pool,
            size_of::<[u32; PAGE]>(),
            remaining_after_dir,
        )?;
        Ok(PagedPlan {
            dir_target,
            dir_delta,
            pool_target,
            pool_delta,
        })
    }

    fn plan_reserve_for_nodes(
        &self,
        max_node_index: usize,
        expected_rows: usize,
        budget: &SessionBudget,
    ) -> Result<PagedPlan, WireError> {
        let dir_pages = max_node_index.div_ceil(PAGE).max(1);
        // Contiguous scenes touch `expected_rows.div_ceil(PAGE)` pages (preserving O(1) Frame-0
        // alloc parity), while stride-sparse scenes grow additional pages on demand in `try_set`
        // under `SessionBudget` — keeping bulk pre-reservation <= 0.5x wire bytes even under
        // repeated patches on one high-index node.
        let pool_pages = expected_rows.div_ceil(PAGE).min(dir_pages).max(1);

        let dir_old_cap = self.dir.capacity();
        let pool_old_cap = self.pool.capacity();
        let dir_delta = budget.preview_grow(dir_old_cap, dir_pages, size_of::<u32>())?;
        let pool_delta = budget.preview_grow(pool_old_cap, pool_pages, size_of::<[u32; PAGE]>())?;
        let total_delta = dir_delta.checked_add(pool_delta).ok_or(WireError::Budget)?;
        if total_delta > budget.remaining() {
            return Err(WireError::Budget);
        }
        Ok(PagedPlan {
            dir_target: dir_pages.max(dir_old_cap),
            dir_delta,
            pool_target: pool_pages.max(pool_old_cap),
            pool_delta,
        })
    }

    fn apply_reserve_plan(
        &mut self,
        plan: PagedPlan,
        budget: &mut SessionBudget,
    ) -> Result<(), WireError> {
        if plan.dir_delta > 0 {
            self.dir
                .try_reserve_exact(plan.dir_target - self.dir.len())
                .map_err(|_| WireError::Alloc)?;
            budget.charge(plan.dir_delta)?;
        }
        if plan.pool_delta > 0 {
            self.pool
                .try_reserve_exact(plan.pool_target - self.pool.len())
                .map_err(|_| WireError::Alloc)?;
            budget.charge(plan.pool_delta)?;
        }
        Ok(())
    }

    fn apply_try_set_plan(
        &mut self,
        plan: PagedPlan,
        node: NodeIndex,
        row_plus_one: u32,
        budget: &mut SessionBudget,
    ) -> Result<(), WireError> {
        self.apply_reserve_plan(plan, budget)?;
        let n = node.0;
        let p = (n >> PAGE_BITS) as usize;
        let needed_dir = p + 1;
        if self.dir.len() < needed_dir {
            self.dir.resize(needed_dir, NO_PAGE);
        }
        if self.dir[p] == NO_PAGE {
            self.dir[p] = self.pool.len() as u32;
            self.pool.push([0; PAGE]);
        }
        self.pool[self.dir[p] as usize][n as usize & (PAGE - 1)] = row_plus_one;
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn try_set(
        &mut self,
        node: NodeIndex,
        row_plus_one: u32,
        budget: &mut SessionBudget,
    ) -> Result<(), WireError> {
        let plan = self.plan_try_set(node, budget.remaining())?;
        self.apply_try_set_plan(plan, node, row_plus_one, budget)
    }

    #[cfg(test)]
    pub(crate) fn with_exact_caps(dir_len: usize, pool_len: usize) -> Self {
        let mut dir = Vec::new();
        dir.reserve_exact(dir_len);
        dir.resize(dir_len, NO_PAGE);
        let mut pool = Vec::new();
        pool.reserve_exact(pool_len);
        pool.resize(pool_len, [0; PAGE]);
        Self { dir, pool }
    }

    #[cfg(test)]
    pub(crate) fn dir_cap(&self) -> usize {
        self.dir.capacity()
    }

    #[cfg(test)]
    pub(crate) fn pool_cap(&self) -> usize {
        self.pool.capacity()
    }
}

/// Storage + per-frame change tracking for one table in one store.
#[derive(Debug)]
pub(crate) struct Column {
    pub(crate) desc: TableMeta,
    pub(crate) w: usize,
    /// Dense: `node_index * w`; sparse: `row * w`.
    pub(crate) rows: Vec<Word>,
    /// Dirty word mask per row (dense: per node index).
    pub(crate) masks: Vec<u32>,
    /// Dense: `NodeId` per live attached slot (`NodeId::NULL` = unattached); sparse: `NodeId` per row.
    pub(crate) owners: Vec<NodeId>,
    index: PagedIndex,
    /// Node indices whose mask went `0 → non-zero` this frame.
    pub(crate) dirty: DirtyList,
    /// `NodeId`s that detached this table this frame (deduplicated sparsely at `commit_into`).
    pub(crate) detached: Vec<NodeId>,
    /// Packed sparse row indices populated by swap-remove this frame (deduplicated via
    /// `relocated_epoch` in O(1); merged into `NveTableView::dirty_rows` without `NVE2` re-send).
    pub(crate) relocated_rows: Vec<u32>,
    /// Per-row-index stamp preventing duplicate `relocated_rows` entries within one frame.
    pub(crate) relocated_epoch: Vec<u64>,
    pub(crate) frame_epoch: u64,
}

impl Column {
    pub(crate) fn new(desc: TableMeta) -> Self {
        let w = desc.words();
        Self {
            desc,
            w,
            rows: Vec::new(),
            masks: Vec::new(),
            owners: Vec::new(),
            index: PagedIndex::default(),
            dirty: DirtyList::default(),
            detached: Vec::new(),
            relocated_rows: Vec::new(),
            relocated_epoch: Vec::new(),
            frame_epoch: 1,
        }
    }

    #[inline]
    pub(crate) fn record_detached(&mut self, id: NodeId) {
        self.detached.push(id);
    }

    pub(crate) fn next_frame_epoch(&mut self) {
        self.frame_epoch = self.frame_epoch.wrapping_add(1);
        if self.frame_epoch == 0 {
            self.relocated_epoch.fill(0);
            self.frame_epoch = 1;
        }
    }

    /// Only compiled static tables declared `Storage::Dense` use dense node-indexed storage.
    /// Wire-declared dynamic tables always use paged-sparse storage in memory while keeping
    /// `TableMeta::storage()` faithful to the wire schema for downstream relaying.
    #[inline(always)]
    pub(crate) fn is_dense(&self) -> bool {
        matches!(self.desc, TableMeta::Static(d) if matches!(d.storage, Storage::Dense))
    }

    /// Minimum byte delta required across `masks`, `owners`, and `rows` to hold `needed` rows.
    fn min_row_buffers_delta(&self, needed: usize) -> Result<usize, WireError> {
        let needed_words = needed.checked_mul(self.w).ok_or(WireError::Budget)?;
        let masks_min = needed
            .saturating_sub(self.masks.capacity())
            .checked_mul(size_of::<u32>())
            .ok_or(WireError::Budget)?;
        let owners_min = needed
            .saturating_sub(self.owners.capacity())
            .checked_mul(size_of::<u32>())
            .ok_or(WireError::Budget)?;
        let rows_min = needed_words
            .saturating_sub(self.rows.capacity())
            .checked_mul(size_of::<Word>())
            .ok_or(WireError::Budget)?;
        masks_min
            .checked_add(owners_min)
            .and_then(|n| n.checked_add(rows_min))
            .ok_or(WireError::Budget)
    }

    /// Plans `masks` + `owners` + `rows` capacity growth for `needed` rows against `remaining_bytes`
    /// without mutating or allocating, deriving `old_cap` from the minimum across all three buffers
    /// so partial-OOM recovery never double-charges or skews budget accounting.
    fn plan_row_buffers(
        &self,
        needed: usize,
        remaining_bytes: usize,
    ) -> Result<RowBufferPlan, WireError> {
        let masks_old = self.masks.capacity();
        let owners_old = self.owners.capacity();
        let rows_old = self.rows.capacity() / self.w;
        let old_cap = masks_old.min(owners_old).min(rows_old);
        if needed <= old_cap {
            return Ok(RowBufferPlan {
                target_cap: old_cap,
                target_words: old_cap * self.w,
                masks_delta: 0,
                owners_delta: 0,
                rows_delta: 0,
            });
        }
        let bytes_per_slot = self
            .w
            .checked_add(2)
            .and_then(|words| words.checked_mul(size_of::<u32>()))
            .ok_or(WireError::Budget)?;
        let (target_cap, _) = SessionBudget::bounded_target_cap(
            old_cap,
            self.masks.len(),
            needed,
            bytes_per_slot,
            remaining_bytes,
        )?;
        let target_words = target_cap.checked_mul(self.w).ok_or(WireError::Budget)?;
        let masks_delta = target_cap
            .saturating_sub(masks_old)
            .checked_mul(size_of::<u32>())
            .ok_or(WireError::Budget)?;
        let owners_delta = target_cap
            .saturating_sub(owners_old)
            .checked_mul(size_of::<u32>())
            .ok_or(WireError::Budget)?;
        let rows_delta = target_words
            .saturating_sub(self.rows.capacity())
            .checked_mul(size_of::<Word>())
            .ok_or(WireError::Budget)?;
        let total_delta = masks_delta
            .checked_add(owners_delta)
            .and_then(|n| n.checked_add(rows_delta))
            .ok_or(WireError::Budget)?;
        if total_delta > remaining_bytes {
            return Err(WireError::Budget);
        }
        Ok(RowBufferPlan {
            target_cap,
            target_words,
            masks_delta,
            owners_delta,
            rows_delta,
        })
    }

    /// Executes a validated [`RowBufferPlan`], charging each buffer immediately upon reservation so
    /// partial OOM never desynchronizes `SessionBudget` from actual heap capacity.
    fn apply_row_buffers_plan(
        &mut self,
        plan: RowBufferPlan,
        budget: &mut SessionBudget,
    ) -> Result<(), WireError> {
        if plan.masks_delta > 0 {
            self.masks
                .try_reserve_exact(plan.target_cap - self.masks.len())
                .map_err(|_| WireError::Alloc)?;
            budget.charge(plan.masks_delta)?;
        }
        if plan.owners_delta > 0 {
            self.owners
                .try_reserve_exact(plan.target_cap - self.owners.len())
                .map_err(|_| WireError::Alloc)?;
            budget.charge(plan.owners_delta)?;
        }
        if plan.rows_delta > 0 {
            self.rows
                .try_reserve_exact(plan.target_words - self.rows.len())
                .map_err(|_| WireError::Alloc)?;
            budget.charge(plan.rows_delta)?;
        }
        Ok(())
    }

    /// Fallibly pre-sizes dense/sparse storage for a bulk wire apply so decoding `N` rows performs
    /// O(1) heap allocations independent of `N`, validating and charging all dense/sparse row
    /// buffers and `PagedIndex` growth against `budget` before allocating any buffer.
    pub(crate) fn reserve_bulk(
        &mut self,
        expected_rows: usize,
        max_node_index: usize,
        budget: &mut SessionBudget,
    ) -> Result<(), WireError> {
        if self.is_dense() {
            let cap = max_node_index.max(64).min(NodeId::MAX_INDEX as usize + 1);
            let word_cap = cap.checked_mul(self.w).ok_or(WireError::Budget)?;
            if self.masks.len() < cap {
                let row_plan = self.plan_row_buffers(cap, budget.remaining())?;
                self.apply_row_buffers_plan(row_plan, budget)?;
                self.masks.resize(cap, 0);
                self.owners.resize(cap, NodeId::NULL);
                self.rows.resize(word_cap, 0);
            }
        } else if self.owners.is_empty() {
            let target = expected_rows.min(max_node_index.saturating_add(1));
            let idx_plan = self
                .index
                .plan_reserve_for_nodes(max_node_index, target, budget)?;
            let remaining_after_idx = budget.remaining().saturating_sub(idx_plan.total_delta());
            let row_plan = self.plan_row_buffers(target, remaining_after_idx)?;
            self.index.apply_reserve_plan(idx_plan, budget)?;
            self.apply_row_buffers_plan(row_plan, budget)?;
        }
        let dirty_cap_remaining = max_node_index.saturating_sub(self.dirty.as_slice().len());
        let dirty_additional = expected_rows.min(dirty_cap_remaining);
        self.dirty.try_reserve(dirty_additional)?;
        Ok(())
    }

    /// Row of a node that has this table attached.
    #[inline(always)]
    pub(crate) fn row_of(&self, node_index: NodeIndex) -> usize {
        if self.is_dense() {
            node_index.as_usize()
        } else {
            self.index.get(node_index).unwrap_or(u32::MAX) as usize
        }
    }

    /// Row of `node_index` if this sparse column holds one (dense: always `Some`).
    #[inline]
    pub(crate) fn find_row(&self, node_index: NodeIndex) -> Option<usize> {
        if self.is_dense() {
            Some(node_index.as_usize())
        } else {
            self.index.get(node_index).map(|r| r as usize)
        }
    }

    #[inline(always)]
    pub(crate) fn words(&self, row: usize) -> &[Word] {
        &self.rows[row * self.w..(row + 1) * self.w]
    }

    /// Creates the row of `id` with default words and a full dirty mask; returns the row.
    pub(crate) fn attach(&mut self, id: NodeId, node_index: NodeIndex) -> usize {
        let full = full_mask(self.w);
        let row = if self.is_dense() {
            let n = node_index.as_usize();
            if self.masks.len() <= n {
                let cap = (n + 1).next_power_of_two().max(64);
                self.masks.resize(cap, 0);
                self.owners.resize(cap, NodeId::NULL);
                self.rows.resize(cap * self.w, 0);
            }
            self.owners[n] = id;
            n
        } else {
            let r = self.owners.len();
            self.owners.push(id);
            self.masks.push(0);
            self.rows.resize((r + 1) * self.w, 0);
            self.index.set(node_index, r as u32 + 1);
            r
        };
        self.rows[row * self.w..(row + 1) * self.w].copy_from_slice(self.desc.defaults());
        let was = self.masks[row];
        self.masks[row] = full;
        self.dirty.push_if(node_index, was == 0);
        row
    }

    /// Fallible variant of [`Self::attach`] used by [`crate::wire::WireDecoder`] so multi-op
    /// sparse frames never fall through to infallible `Vec::push`/`resize` growth.
    ///
    /// Dense columns must already be pre-sized through [`Self::reserve_bulk`] (session budget);
    /// an under-sized dense column is rejected with [`WireError::Budget`] rather than growing
    /// outside the budget.
    pub(crate) fn try_attach(
        &mut self,
        id: NodeId,
        node_index: NodeIndex,
        budget: &mut SessionBudget,
    ) -> Result<usize, WireError> {
        let full = full_mask(self.w);
        let row = if self.is_dense() {
            let n = node_index.as_usize();
            debug_assert!(
                n < self.masks.len(),
                "dense column must be pre-sized via reserve_bulk before try_attach"
            );
            if n >= self.masks.len() {
                return Err(WireError::Budget);
            }
            self.dirty.try_reserve(1)?;
            self.owners[n] = id;
            n
        } else {
            let r = self.owners.len();
            let needed_rows = r.checked_add(1).ok_or(WireError::Budget)?;
            let next_rows = needed_rows.checked_mul(self.w).ok_or(WireError::Budget)?;
            let min_row_delta = self.min_row_buffers_delta(needed_rows)?;
            if min_row_delta > budget.remaining() {
                return Err(WireError::Budget);
            }
            let idx_plan = self
                .index
                .plan_try_set(node_index, budget.remaining() - min_row_delta)?;
            let remaining_after_idx = budget.remaining().saturating_sub(idx_plan.total_delta());
            let row_plan = self.plan_row_buffers(needed_rows, remaining_after_idx)?;
            self.dirty.try_reserve(1)?;
            self.apply_row_buffers_plan(row_plan, budget)?;
            self.index
                .apply_try_set_plan(idx_plan, node_index, r as u32 + 1, budget)?;
            self.owners.push(id);
            self.masks.push(0);
            self.rows.resize(next_rows, 0);
            r
        };
        self.rows[row * self.w..(row + 1) * self.w].copy_from_slice(self.desc.defaults());
        let was = self.masks[row];
        self.masks[row] = full;
        self.dirty.push_if(node_index, was == 0);
        Ok(row)
    }

    /// Removes the row of `node_index` (dense: clears owner; sparse: swap-remove preserving the
    /// moved tail row's existing dirty mask for `NVE2` while recording `r` in `relocated_rows` so
    /// published `NveTableView::dirty_rows` still notifies index-based caches).
    pub(crate) fn remove_row(&mut self, node_index: NodeIndex) {
        if self.is_dense() {
            let n = node_index.as_usize();
            if n < self.owners.len() {
                self.owners[n] = NodeId::NULL;
            }
            return;
        }
        let Some(r) = self.index.get(node_index) else {
            return;
        };
        let r = r as usize;
        let last = self.owners.len() - 1;
        if r != last {
            let w = self.w;
            self.rows.copy_within(last * w..(last + 1) * w, r * w);
            self.owners[r] = self.owners[last];
            let was = self.masks[last];
            self.masks[r] = was;
            let moved = self.owners[r].node_index();
            self.index.set(moved, r as u32 + 1);
            if self.desc.flags() & nve_abi::table::table_flags::PUBLISH_TO_SCENE != 0 {
                if self.relocated_epoch.len() <= r {
                    self.relocated_epoch.resize(r + 1, 0);
                }
                if self.relocated_epoch[r] != self.frame_epoch {
                    self.relocated_epoch[r] = self.frame_epoch;
                    self.relocated_rows.push(r as u32);
                }
            }
        }
        self.owners.pop();
        self.masks.pop();
        self.rows.truncate(last * self.w);
        self.index.set(node_index, 0);
    }

    /// Rows published in `NveTableView`: dense tables stop at the store's node capacity (the
    /// power-of-two growth tail belongs to no node) and publish no owners (row = node index).
    pub(crate) fn published_row_count(&self, capacity: usize) -> usize {
        if self.is_dense() { self.row_count().min(capacity) } else { self.row_count() }
    }

    /// Number of addressable rows (dense: node capacity; sparse: packed rows).
    #[inline]
    pub(crate) fn row_count(&self) -> usize {
        self.masks.len()
    }

    /// The paged node-index → `row + 1` index as `(directory, flattened pages)` (dense: empty).
    #[inline]
    pub(crate) fn row_index(&self) -> (&[u32], &[u32]) {
        (&self.index.dir, self.index.pool.as_flattened())
    }

    #[cfg(test)]
    #[inline]
    pub(crate) fn paged_pool_cap(&self) -> usize {
        self.index.pool.capacity()
    }

    #[cfg(test)]
    #[inline]
    pub(crate) fn owners_cap(&self) -> usize {
        self.owners.capacity()
    }

    #[cfg(test)]
    #[inline]
    pub(crate) fn masks_cap(&self) -> usize {
        self.masks.capacity()
    }

    #[cfg(test)]
    #[inline]
    pub(crate) fn rows_cap(&self) -> usize {
        self.rows.capacity()
    }
}

#[inline(always)]
pub(crate) const fn full_mask(words: usize) -> u32 {
    if words >= 32 {
        u32::MAX
    } else {
        (1u32 << words) - 1
    }
}
