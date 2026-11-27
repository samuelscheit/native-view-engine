//! One table's storage: dense (row = node index) or paged-sparse (packed rows + owners).

use nve_abi::table::{Storage, Word};

use crate::id::{NodeId, NodeIndex};
use crate::row::DirtyList;
use crate::table::TableMeta;
use crate::wire::WireError;

const PAGE_BITS: u32 = 8;
const PAGE: usize = 1 << PAGE_BITS;
const NO_PAGE: u32 = u32::MAX;

/// Node index → packed row (`row + 1`, `0` = absent) in 256-entry pages; no per-node `Box`.
#[derive(Debug, Default)]
struct PagedIndex {
    dir: Vec<u32>,
    pool: Vec<[u32; PAGE]>,
}

impl PagedIndex {
    #[inline(always)]
    fn get(&self, node: NodeIndex) -> Option<u32> {
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

    fn try_set(&mut self, node: NodeIndex, row_plus_one: u32) -> Result<(), WireError> {
        let n = node.0;
        let p = (n >> PAGE_BITS) as usize;
        if self.dir.len() <= p {
            let needed = p.checked_add(1).ok_or(WireError::Alloc)?;
            self.dir
                .try_reserve(needed - self.dir.len())
                .map_err(|_| WireError::Alloc)?;
            self.dir.resize(needed, NO_PAGE);
        }
        if self.dir[p] == NO_PAGE {
            self.pool.try_reserve(1).map_err(|_| WireError::Alloc)?;
            self.dir[p] = self.pool.len() as u32;
            self.pool.push([0; PAGE]);
        }
        self.pool[self.dir[p] as usize][n as usize & (PAGE - 1)] = row_plus_one;
        Ok(())
    }

    fn reserve_for_nodes(
        &mut self,
        max_node_index: usize,
        expected_rows: usize,
    ) -> Result<(), WireError> {
        let dir_pages = max_node_index.div_ceil(PAGE).max(1);
        let pool_pages = expected_rows.min(dir_pages).max(1);
        if self.dir.capacity() < dir_pages {
            self.dir
                .try_reserve(dir_pages.saturating_sub(self.dir.len()))
                .map_err(|_| WireError::Alloc)?;
        }
        if self.pool.capacity() < pool_pages {
            self.pool
                .try_reserve(pool_pages.saturating_sub(self.pool.len()))
                .map_err(|_| WireError::Alloc)?;
        }
        Ok(())
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
    /// Dense: raw `NodeId` per live attached slot (`0` = unattached); sparse: raw `NodeId` per row.
    pub(crate) owners: Vec<u32>,
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

    /// Fallibly pre-sizes dense/sparse storage for a bulk wire apply so decoding `N` rows performs
    /// O(1) heap allocations independent of `N`, charging exact dense slot delta bytes against the
    /// session slot-memory budget (`WireError::Budget`) and propagating `WireError::Alloc` on OOM.
    pub(crate) fn reserve_bulk(
        &mut self,
        expected_rows: usize,
        max_node_index: usize,
        slot_bytes_used: &mut usize,
        slot_bytes_limit: usize,
    ) -> Result<(), WireError> {
        if self.is_dense() {
            let cap = max_node_index.max(64).min(NodeId::MAX_INDEX as usize + 1);
            let word_cap = cap.checked_mul(self.w).ok_or(WireError::Budget)?;
            if self.masks.len() < cap {
                let bytes_per_slot = self
                    .w
                    .checked_add(2)
                    .and_then(|words| words.checked_mul(size_of::<u32>()))
                    .ok_or(WireError::Budget)?;
                let delta_bytes = (cap - self.masks.len())
                    .checked_mul(bytes_per_slot)
                    .ok_or(WireError::Budget)?;
                let next_used = slot_bytes_used
                    .checked_add(delta_bytes)
                    .ok_or(WireError::Budget)?;
                if next_used > slot_bytes_limit {
                    return Err(WireError::Budget);
                }
                self.masks
                    .try_reserve(cap - self.masks.len())
                    .map_err(|_| WireError::Alloc)?;
                self.owners
                    .try_reserve(cap - self.owners.len())
                    .map_err(|_| WireError::Alloc)?;
                self.rows
                    .try_reserve(word_cap.saturating_sub(self.rows.len()))
                    .map_err(|_| WireError::Alloc)?;
                self.masks.resize(cap, 0);
                self.owners.resize(cap, 0);
                self.rows.resize(word_cap, 0);
                *slot_bytes_used = next_used;
            }
        } else if self.owners.is_empty() {
            let target = expected_rows.min(max_node_index.saturating_add(1));
            if self.owners.capacity() < target {
                self.owners
                    .try_reserve(target)
                    .map_err(|_| WireError::Alloc)?;
            }
            if self.masks.capacity() < target {
                self.masks
                    .try_reserve(target)
                    .map_err(|_| WireError::Alloc)?;
            }
            let word_target = target.checked_mul(self.w).ok_or(WireError::Alloc)?;
            if self.rows.capacity() < word_target {
                self.rows
                    .try_reserve(word_target)
                    .map_err(|_| WireError::Alloc)?;
            }
            self.index.reserve_for_nodes(max_node_index, target)?;
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
                self.owners.resize(cap, 0);
                self.rows.resize(cap * self.w, 0);
            }
            self.owners[n] = id.raw();
            n
        } else {
            let r = self.owners.len();
            self.owners.push(id.raw());
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
            self.owners[n] = id.raw();
            n
        } else {
            let r = self.owners.len();
            let next_rows = r
                .checked_add(1)
                .and_then(|n| n.checked_mul(self.w))
                .ok_or(WireError::Alloc)?;
            self.owners.try_reserve(1).map_err(|_| WireError::Alloc)?;
            self.masks.try_reserve(1).map_err(|_| WireError::Alloc)?;
            self.rows
                .try_reserve(next_rows.saturating_sub(self.rows.len()))
                .map_err(|_| WireError::Alloc)?;
            self.dirty.try_reserve(1)?;
            self.index.try_set(node_index, r as u32 + 1)?;
            self.owners.push(id.raw());
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
                self.owners[n] = 0;
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
            let moved = NodeId(self.owners[r]).node_index();
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

    /// Number of addressable rows (dense: node capacity; sparse: packed rows).
    #[inline]
    pub(crate) fn row_count(&self) -> usize {
        self.masks.len()
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
