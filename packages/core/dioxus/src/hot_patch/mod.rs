//! Targeted Tier-1 binary `RowPatch` & text hot-patching directly over `NodeStore` with
//! collision-free `(template_key, node_path)` resolution and zero steady-state heap allocations.
//!
//! - Compile-time `nve_dioxus::rsx!` stamps static `"__nve_tpl": "<rel_file>#<ordinal>@<path>"`
//!   attributes in debug builds; `NveMutationWriter::instantiate_template_node` strips them before
//!   attribute dispatch and registers `(template_key, prefix_path) -> NodeId` in [`HotPatchIndex`].
//! - Unmounted nodes are evicted in O(1) via the reverse `NodeSlotTable<(u64, u32)>`.
//! - `RowPatchView::from_words` validates the flat `u32` payload and `Transaction::apply_row_patch`
//!   patches packed words directly without CSS string parsing or heap allocation.

mod apply;

use core::ops::Range;

use nve_abi::hash::HashMap;
use nve_engine::{hot_patch_template_key, HotPatchMsg};
use nve_store::NodeId;

use crate::writer::node_slot_table::NodeSlotTable;
use crate::writer::StoreMutationTx;

/// Debug-only static attribute injected by `nve_dioxus::rsx!` and stripped during template mount.
pub const NVE_TEMPLATE_ATTR: &str = "__nve_tpl";

/// Maximum inline anchor path length stored without heap allocation in [`AnchorPath`].
const MAX_ANCHOR_PATH: usize = 8;

/// Maximum queued hot patches retained before rejecting overflow (forces Tier-2 fallback).
pub(crate) const MAX_QUEUED_PATCHES: usize = 256;

/// Maximum total arena bytes retained in [`PendingHotPatchQueue`] before rejecting overflow.
pub(crate) const MAX_QUEUE_ARENA_BYTES: usize = 256 * 1024;

/// Maximum retained backing capacities after [`PendingHotPatchQueue::clear`] drains a burst.
const MAX_RETAINED_ENTRIES_CAP: usize = 32;
const MAX_RETAINED_ARENA_BYTES: usize = 16 * 1024;

/// Anchor path from the template root: inline zero-alloc up to depth 8, heap-backed for deeper
/// subtrees so anchors at any nesting depth resolve cleanly without silent fallback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AnchorPath {
    Inline { len: u8, buf: [u16; MAX_ANCHOR_PATH] },
    Overflow(Box<[u16]>),
}

impl AnchorPath {
    #[inline]
    pub(crate) fn as_slice(&self) -> &[u16] {
        match self {
            Self::Inline { len, buf } => &buf[..usize::from(*len)],
            Self::Overflow(boxed) => boxed,
        }
    }

    #[inline]
    pub(crate) fn len(&self) -> usize {
        match self {
            Self::Inline { len, .. } => usize::from(*len),
            Self::Overflow(boxed) => boxed.len(),
        }
    }
}

/// Live mounted template anchor mapping a static `(template_key, prefix_path)` to one live `NodeId`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TemplateRootEntry {
    pub(crate) node: NodeId,
    pub(crate) path: AnchorPath,
}

impl TemplateRootEntry {
    #[inline]
    pub(crate) fn prefix(&self) -> &[u16] {
        self.path.as_slice()
    }

    #[inline]
    pub(crate) fn path_len(&self) -> usize {
        self.path.len()
    }
}

/// Reusable per-writer index of mounted template anchors plus zero-alloc hot-patch scratch buffers.
#[derive(Debug, Default)]
pub(crate) struct HotPatchIndex {
    pub(crate) template_roots: HashMap<u64, Vec<TemplateRootEntry>>,
    /// Reverse index `(template_key, slot_index_in_template_roots_vec)` for O(1) swap-remove unbind.
    pub(crate) node_template_keys: NodeSlotTable<(u64, u32)>,
    pub(crate) targets: Vec<NodeId>,
    pub(crate) stack: Vec<NodeId>,
    pub(crate) candidates: Vec<NodeId>,
    pub(crate) matched_text: Vec<NodeId>,
}

impl HotPatchIndex {
    /// Zero eager heap allocations: scratch buffers grow lazily only when hot-patching is active.
    #[inline]
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn reset(&mut self) {
        self.template_roots.clear();
        self.node_template_keys.reset();
        self.targets.clear();
        self.stack.clear();
        self.candidates.clear();
        self.matched_text.clear();
    }

    /// Parses `"<rel_file>#<ordinal>@<p0>/<p1>..."` (zero-alloc up to depth 8) and registers `node`.
    pub(crate) fn register_tag(&mut self, node: NodeId, raw: &str) {
        let Some((key, path)) = parse_template_tag(raw) else {
            return;
        };
        if self.node_template_keys.get(node).is_some() {
            self.unbind_node(node);
        }
        let entries = self.template_roots.entry(key).or_default();
        let slot = entries.len() as u32;
        entries.push(TemplateRootEntry { node, path });
        self.node_template_keys.insert(node, (key, slot));
    }

    /// Evicts `node` from the live template index in O(1) when its subtree is unmounted.
    #[inline]
    pub(crate) fn unbind_node(&mut self, node: NodeId) {
        let Some((key, slot)) = self.node_template_keys.remove(node) else {
            return;
        };
        if let Some(entries) = self.template_roots.get_mut(&key) {
            let idx = slot as usize;
            if entries.get(idx).is_some_and(|e| e.node == node) {
                entries.swap_remove(idx);
                if let Some(moved) = entries.get(idx)
                    && let Some(meta) = self.node_template_keys.get_mut(moved.node)
                {
                    meta.1 = slot;
                }
            }
        }
    }
}

/// Parses `"<rel_file>#<ordinal>@<p0>/<p1>..."` (inline zero-alloc for depth `1..=8`, boxed for deeper).
pub(crate) fn parse_template_tag(raw: &str) -> Option<(u64, AnchorPath)> {
    let (left, path_str) = raw.rsplit_once('@')?;
    let (rel_file, ord_str) = left.rsplit_once('#')?;
    let ordinal: u32 = ord_str.parse().ok()?;
    let mut inline_buf = [0u16; MAX_ANCHOR_PATH];
    let mut inline_len = 0usize;
    let mut overflow: Option<Vec<u16>> = None;
    for seg in path_str.split('/') {
        let idx = seg.parse::<u16>().ok()?;
        if let Some(vec) = overflow.as_mut() {
            vec.push(idx);
        } else if inline_len < MAX_ANCHOR_PATH {
            inline_buf[inline_len] = idx;
            inline_len += 1;
        } else {
            let mut vec = Vec::with_capacity(MAX_ANCHOR_PATH + 4);
            vec.extend_from_slice(&inline_buf);
            vec.push(idx);
            overflow = Some(vec);
        }
    }
    let path = match overflow {
        Some(vec) => AnchorPath::Overflow(vec.into_boxed_slice()),
        None if inline_len > 0 => AnchorPath::Inline {
            len: inline_len as u8,
            buf: inline_buf,
        },
        None => return None,
    };
    let key = hot_patch_template_key(rel_file, ordinal);
    Some((key, path))
}

#[derive(Debug, Clone)]
struct QueuedPatchMeta {
    template_key: u64,
    node_path: Range<usize>,
    target_test_id: Option<Range<usize>>,
    row_patch_words: Range<usize>,
    text_updates: Range<usize>,
    text_replacements: Range<usize>,
}

/// Flat bounded reusable arena for pending [`HotPatchMsg::TemplatePatch`]es on
/// [`crate::DioxusSurface`]. Once warmed, pushing and draining patches performs **zero** heap
/// allocations while capping queued entries/bytes and shrinking post-burst spikes.
#[derive(Debug, Default)]
pub(crate) struct PendingHotPatchQueue {
    entries: Vec<QueuedPatchMeta>,
    node_paths: Vec<u16>,
    row_patch_words: Vec<u32>,
    test_ids: String,
    text_updates: Vec<(usize, Range<usize>)>,
    text_replacements: Vec<(Range<usize>, Range<usize>)>,
    text_arena: String,
}

impl PendingHotPatchQueue {
    #[inline]
    fn arena_bytes(&self) -> usize {
        self.node_paths.len() * core::mem::size_of::<u16>()
            + self.row_patch_words.len() * core::mem::size_of::<u32>()
            + self.test_ids.len()
            + self.text_updates.len() * core::mem::size_of::<(usize, Range<usize>)>()
            + self.text_replacements.len() * core::mem::size_of::<(Range<usize>, Range<usize>)>()
            + self.text_arena.len()
    }

    pub(crate) fn push(&mut self, patch: &HotPatchMsg) -> bool {
        let HotPatchMsg::TemplatePatch {
            template_key,
            node_path,
            target_test_id,
            row_patch_words,
            text_updates,
            text_replacements,
            ..
        } = patch
        else {
            return false;
        };

        let incoming_text: usize = text_updates.iter().map(|(_, t)| t.len()).sum::<usize>()
            + text_replacements
                .iter()
                .map(|(o, n)| o.len() + n.len())
                .sum::<usize>();
        let incoming_bytes = node_path
            .len()
            .saturating_mul(core::mem::size_of::<u16>())
            .saturating_add(
                row_patch_words
                    .len()
                    .saturating_mul(core::mem::size_of::<u32>()),
            )
            .saturating_add(target_test_id.as_deref().map_or(0, str::len))
            .saturating_add(
                text_updates
                    .len()
                    .saturating_mul(core::mem::size_of::<(usize, Range<usize>)>()),
            )
            .saturating_add(
                text_replacements
                    .len()
                    .saturating_mul(core::mem::size_of::<(Range<usize>, Range<usize>)>()),
            )
            .saturating_add(incoming_text);
        if self.entries.len() >= MAX_QUEUED_PATCHES
            || self.arena_bytes().saturating_add(incoming_bytes) > MAX_QUEUE_ARENA_BYTES
        {
            return false;
        }

        let np_start = self.node_paths.len();
        self.node_paths.extend_from_slice(node_path);
        let np_range = np_start..self.node_paths.len();

        let tid_range = target_test_id.as_deref().map(|tid| {
            let s = self.test_ids.len();
            self.test_ids.push_str(tid);
            s..self.test_ids.len()
        });

        let rp_start = self.row_patch_words.len();
        self.row_patch_words.extend_from_slice(row_patch_words);
        let rp_range = rp_start..self.row_patch_words.len();

        let tu_start = self.text_updates.len();
        for (idx, text) in text_updates {
            let s = self.text_arena.len();
            self.text_arena.push_str(text);
            self.text_updates.push((*idx, s..self.text_arena.len()));
        }
        let tu_range = tu_start..self.text_updates.len();

        let tr_start = self.text_replacements.len();
        for (old_t, new_t) in text_replacements {
            let os = self.text_arena.len();
            self.text_arena.push_str(old_t);
            let oe = self.text_arena.len();
            self.text_arena.push_str(new_t);
            let ne = self.text_arena.len();
            self.text_replacements.push((os..oe, oe..ne));
        }
        let tr_range = tr_start..self.text_replacements.len();

        self.entries.push(QueuedPatchMeta {
            template_key: *template_key,
            node_path: np_range,
            target_test_id: tid_range,
            row_patch_words: rp_range,
            text_updates: tu_range,
            text_replacements: tr_range,
        });
        true
    }

    pub(crate) fn apply_all(&mut self, mtx: &mut StoreMutationTx<'_, '_>) -> bool {
        if self.entries.is_empty() {
            return false;
        }
        let mut changed = false;
        for entry in &self.entries {
            let node_path = &self.node_paths[entry.node_path.clone()];
            let target_test_id = entry
                .target_test_id
                .as_ref()
                .map(|r| &self.test_ids[r.clone()]);
            let row_patch_words = &self.row_patch_words[entry.row_patch_words.clone()];
            let tu_slice = &self.text_updates[entry.text_updates.clone()];
            let tr_slice = &self.text_replacements[entry.text_replacements.clone()];
            let text_arena = &self.text_arena;
            changed |= mtx.apply_hot_patch_inner(
                entry.template_key,
                node_path,
                target_test_id,
                row_patch_words,
                tu_slice.len(),
                |i| {
                    let (idx, ref r) = tu_slice[i];
                    (idx, &text_arena[r.clone()])
                },
                tr_slice.len(),
                |i| {
                    let (ref old_r, ref new_r) = tr_slice[i];
                    (&text_arena[old_r.clone()], &text_arena[new_r.clone()])
                },
            );
        }
        self.clear();
        changed
    }

    pub(crate) fn clear(&mut self) {
        self.entries.clear();
        self.node_paths.clear();
        self.row_patch_words.clear();
        self.test_ids.clear();
        self.text_updates.clear();
        self.text_replacements.clear();
        self.text_arena.clear();
        if self.entries.capacity() > MAX_RETAINED_ENTRIES_CAP {
            self.entries.shrink_to(MAX_RETAINED_ENTRIES_CAP);
        }
        let u16_cap = MAX_RETAINED_ARENA_BYTES / core::mem::size_of::<u16>();
        if self.node_paths.capacity() > u16_cap {
            self.node_paths.shrink_to(u16_cap);
        }
        let u32_cap = MAX_RETAINED_ARENA_BYTES / core::mem::size_of::<u32>();
        if self.row_patch_words.capacity() > u32_cap {
            self.row_patch_words.shrink_to(u32_cap);
        }
        if self.test_ids.capacity() > MAX_RETAINED_ARENA_BYTES {
            self.test_ids.shrink_to(MAX_RETAINED_ARENA_BYTES);
        }
        if self.text_updates.capacity() > MAX_RETAINED_ENTRIES_CAP {
            self.text_updates.shrink_to(MAX_RETAINED_ENTRIES_CAP);
        }
        if self.text_replacements.capacity() > MAX_RETAINED_ENTRIES_CAP {
            self.text_replacements.shrink_to(MAX_RETAINED_ENTRIES_CAP);
        }
        if self.text_arena.capacity() > MAX_RETAINED_ARENA_BYTES {
            self.text_arena.shrink_to(MAX_RETAINED_ARENA_BYTES);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hot_patch_index_o1_swap_remove_keeps_back_indices_exact_and_supports_deep_anchors() {
        let mut idx = HotPatchIndex::new();
        assert_eq!(idx.targets.capacity(), 0);
        assert_eq!(idx.stack.capacity(), 0);
        assert_eq!(idx.candidates.capacity(), 0);
        assert_eq!(idx.matched_text.capacity(), 0);

        let nodes = [
            NodeId::new(1, 1),
            NodeId::new(2, 1),
            NodeId::new(3, 1),
            NodeId::new(4, 1),
        ];
        let tag = "app.rs#0@0/1/2/3/4/5/6/7/8/9";
        for &n in &nodes {
            idx.register_tag(n, tag);
        }
        let (key, parsed_path) = parse_template_tag(tag).expect("deep anchor path parses");
        assert_eq!(parsed_path.as_slice(), &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);
        assert!(matches!(parsed_path, AnchorPath::Overflow(_)));

        // Unbind first, middle, last, and remaining in turn — back-indices must stay exact.
        idx.unbind_node(nodes[0]);
        assert_eq!(idx.template_roots[&key].len(), 3);
        idx.unbind_node(nodes[2]);
        assert_eq!(idx.template_roots[&key].len(), 2);
        idx.unbind_node(nodes[3]);
        assert_eq!(idx.template_roots[&key].len(), 1);
        idx.unbind_node(nodes[1]);
        assert!(idx.template_roots[&key].is_empty());
        // Capacity stays retained for zero-alloc remount until `reset()`.
        assert!(idx.template_roots[&key].capacity() >= 4);
    }

    #[test]
    fn pending_hot_patch_queue_enforces_entry_and_byte_caps_and_shrinks_bursts() {
        let mut q = PendingHotPatchQueue::default();
        let tiny = HotPatchMsg::TemplatePatch {
            generation: 1,
            template_key: 42,
            node_path: vec![0],
            target_test_id: None,
            row_patch_words: vec![0; 4],
            text_updates: vec![],
            text_replacements: vec![],
        };
        for _ in 0..MAX_QUEUED_PATCHES {
            assert!(q.push(&tiny));
        }
        assert!(!q.push(&tiny), "must reject once MAX_QUEUED_PATCHES is reached");
        q.clear();
        assert!(
            q.entries.capacity() <= MAX_RETAINED_ENTRIES_CAP * 2,
            "oversized entry capacity should shrink after clear: {}",
            q.entries.capacity()
        );

        // Single huge patch exceeding MAX_QUEUE_ARENA_BYTES is rejected without mutating arenas.
        let huge = HotPatchMsg::TemplatePatch {
            generation: 2,
            template_key: 42,
            node_path: vec![0],
            target_test_id: None,
            row_patch_words: vec![],
            text_updates: vec![],
            text_replacements: vec![("a".into(), "x".repeat(MAX_QUEUE_ARENA_BYTES + 1))],
        };
        assert!(!q.push(&huge));
        assert!(q.entries.is_empty());
        assert!(q.text_arena.is_empty());
    }
}
