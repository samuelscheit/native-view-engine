//! Bound `NveTableView` lookups and zero-boilerplate scalar/string/resource table sync into a
//! replica [`nve_store::NodeStore`].

use nve_abi::table::{
    NveTableView, NVE_ROW_INDEX_NO_PAGE, NVE_ROW_INDEX_PAGE_BITS, NVE_ROW_INDEX_PAGE_LEN,
};
use nve_abi::NveNodeRow;
use nve_codec::ByteReader;
use nve_store::{FlatResource, NodeId, NveTable, RowPatch, StrProp, TxCore, ValidatedWords};
use nve_tables::tables::{
    Accessibility, AriaRelations, DebugId, Image, Interaction, Layout, Link, Scroll, TextContent,
    TextInput, Transform, Visual,
};

use crate::bridge::ResourceReader;

/// Safe bounded slice extraction from a C-ABI pointer + count pair.
///
/// # Safety
///
/// When `len > 0` and `!ptr.is_null()`, `ptr` must point to at least `len` initialized, properly
/// aligned elements of `T` valid for lifetime `'a`.
#[inline]
pub(crate) unsafe fn raw_slice<'a, T>(ptr: *const T, len: u32) -> &'a [T] {
    if len == 0 || ptr.is_null() {
        &[]
    } else {
        // SAFETY: guaranteed by caller's `NveSceneView` validity contract.
        unsafe { core::slice::from_raw_parts(ptr, len as usize) }
    }
}

#[inline]
fn has_scene_slot(row: &NveNodeRow, slot: u32) -> bool {
    slot < 128 && (row.table_set[(slot >> 5) as usize] & (1u32 << (slot & 31))) != 0
}

#[inline]
fn read_slab_str<'a>(text_slab: &'a [u8], words: &[u32], word_idx: u8) -> &'a str {
    let w = word_idx as usize;
    let (Some(&off), Some(&len)) = (words.get(w), words.get(w + 1)) else {
        return "";
    };
    if len == 0 {
        return "";
    }
    let start = off as usize;
    let Some(end) = start.checked_add(len as usize) else {
        return "";
    };
    text_slab
        .get(start..end)
        .and_then(|bytes| core::str::from_utf8(bytes).ok())
        .unwrap_or("")
}

#[derive(Clone, Copy)]
pub(crate) struct TableBinding<'a> {
    pub(crate) scene_slot: u32,
    pub(crate) words: usize,
    pub(crate) row_count: usize,
    pub(crate) rows: &'a [u32],
    pub(crate) owners: &'a [NodeId],
    dir: &'a [u32],
    pages: &'a [u32],
    page_count: u32,
}

impl<'a> TableBinding<'a> {
    /// Binds the `NveTableView` matching `T::DESC` from `tables`, returning `None` if absent or
    /// schema-mismatched.
    ///
    /// # Safety
    ///
    /// Every pointer inside each `NveTableView` in `tables` must satisfy the `NveSceneView` ABI
    /// contract for lifetime `'a`.
    unsafe fn bind<T: NveTable>(tables: &'a [NveTableView]) -> Option<Self> {
        let want_key = T::DESC.key.0;
        let want_words = T::DESC.words();
        for tv in tables {
            let key = (u64::from(tv.key_hi) << 32) | u64::from(tv.key_lo);
            if key != want_key || tv.words as usize != want_words {
                continue;
            }
            let row_count = tv.row_count as usize;
            let total_words = row_count.checked_mul(want_words)?;
            let total_words_u32 = u32::try_from(total_words).ok()?;
            // SAFETY: caller guarantees `tv` slices are valid for `'a`.
            let rows = unsafe { raw_slice(tv.rows, total_words_u32) };
            // SAFETY: dense tables have null `owners`; sparse tables have `row_count` owners.
            let owners = unsafe {
                if tv.owners.is_null() {
                    &[]
                } else {
                    raw_slice(tv.owners, tv.row_count)
                }
            };
            // SAFETY: sparse tables expose `row_index_dir_len` entries; dense tables have 0.
            let dir = unsafe { raw_slice(tv.row_index_dir, tv.row_index_dir_len) };
            let page_entries = tv.row_index_page_count.checked_mul(NVE_ROW_INDEX_PAGE_LEN)?;
            // SAFETY: sparse tables expose `row_index_page_count * PAGE_LEN` entries.
            let pages = unsafe { raw_slice(tv.row_index_pages, page_entries) };
            return Some(Self {
                scene_slot: tv.slot,
                words: want_words,
                row_count,
                rows,
                owners,
                dir,
                pages,
                page_count: tv.row_index_page_count,
            });
        }
        None
    }

    #[inline]
    fn row_words(&self, node_idx: usize, node_id: NodeId) -> Option<&'a [u32]> {
        let r = if self.owners.is_empty() {
            if node_idx < self.row_count {
                node_idx
            } else {
                return None;
            }
        } else {
            let page_idx = node_idx >> NVE_ROW_INDEX_PAGE_BITS;
            let page = *self.dir.get(page_idx)?;
            if page == NVE_ROW_INDEX_NO_PAGE || page >= self.page_count {
                return None;
            }
            let lane = node_idx & ((NVE_ROW_INDEX_PAGE_LEN - 1) as usize);
            let entry = *self
                .pages
                .get((page as usize) * (NVE_ROW_INDEX_PAGE_LEN as usize) + lane)?;
            if entry == 0 {
                return None;
            }
            let row = (entry - 1) as usize;
            if row >= self.row_count || self.owners.get(row).copied() != Some(node_id) {
                return None;
            }
            row
        };
        let start = r.checked_mul(self.words)?;
        let end = start.checked_add(self.words)?;
        self.rows.get(start..end)
    }
}

pub(crate) struct BoundA11yTables<'a> {
    pub(crate) layout: Option<TableBinding<'a>>,
    pub(crate) visual: Option<TableBinding<'a>>,
    pub(crate) transform: Option<TableBinding<'a>>,
    pub(crate) scroll: Option<TableBinding<'a>>,
    pub(crate) interaction: Option<TableBinding<'a>>,
    pub(crate) accessibility: Option<TableBinding<'a>>,
    pub(crate) debug_id: Option<TableBinding<'a>>,
    pub(crate) text_content: Option<TableBinding<'a>>,
    pub(crate) text_input: Option<TableBinding<'a>>,
    pub(crate) link: Option<TableBinding<'a>>,
    pub(crate) image: Option<TableBinding<'a>>,
}

impl<'a> BoundA11yTables<'a> {
    /// # Safety
    ///
    /// `tables` must come from a valid `NveSceneView` for lifetime `'a`.
    pub(crate) unsafe fn bind(tables: &'a [NveTableView]) -> Self {
        // SAFETY: delegated to caller's `NveSceneView` contract.
        unsafe {
            Self {
                layout: TableBinding::bind::<Layout>(tables),
                visual: TableBinding::bind::<Visual>(tables),
                transform: TableBinding::bind::<Transform>(tables),
                scroll: TableBinding::bind::<Scroll>(tables),
                interaction: TableBinding::bind::<Interaction>(tables),
                accessibility: TableBinding::bind::<Accessibility>(tables),
                debug_id: TableBinding::bind::<DebugId>(tables),
                text_content: TableBinding::bind::<TextContent>(tables),
                text_input: TableBinding::bind::<TextInput>(tables),
                link: TableBinding::bind::<Link>(tables),
                image: TableBinding::bind::<Image>(tables),
            }
        }
    }
}

pub(crate) fn sync_children(
    tx: &mut TxCore<'_>,
    id: NodeId,
    row: &NveNodeRow,
    children_slab: &[u32],
    scratch: &mut Vec<NodeId>,
) {
    scratch.clear();
    let off = row.child_off as usize;
    let len = row.child_len as usize;
    if len > 0
        && let Some(end) = off.checked_add(len)
        && let Some(raw_children) = children_slab.get(off..end)
    {
        for &raw in raw_children {
            let cid = NodeId::from_raw_or_null(raw);
            if !cid.is_null() && tx.store().is_live(cid) {
                scratch.push(cid);
            }
        }
    }
    let _ = tx.try_set_children(id, scratch.as_slice());
}

fn sync_table_scalars<'a, T: NveTable>(
    tx: &mut TxCore<'_>,
    id: NodeId,
    node_idx: usize,
    node_row: &NveNodeRow,
    binding: Option<TableBinding<'a>>,
    patch_words: &mut Vec<u32>,
) -> Option<&'a [u32]> {
    let Some(b) = binding else {
        if tx.store().has::<T>(id) {
            tx.detach::<T>(id);
        }
        return None;
    };
    if !has_scene_slot(node_row, b.scene_slot) {
        if tx.store().has::<T>(id) {
            tx.detach::<T>(id);
        }
        return None;
    }
    let Some(words) = b.row_words(node_idx, id) else {
        if tx.store().has::<T>(id) {
            tx.detach::<T>(id);
        }
        return None;
    };

    let desc = &T::DESC;
    let str_mask = desc.str_words | (desc.str_words << 1);
    let skip_mask = str_mask | desc.resource_words;
    patch_words.clear();
    patch_words.extend_from_slice(&[1, desc.key.lo(), desc.key.hi(), 0]);
    let mut op_count = 0u32;
    for (w, &val) in words.iter().enumerate() {
        if (skip_mask & (1u32 << (w & 31))) != 0 {
            continue;
        }
        patch_words.push(w as u32);
        patch_words.push(u32::MAX);
        patch_words.push(val);
        op_count += 1;
    }
    if op_count > 0 {
        patch_words[3] = op_count;
        if let Some(validated) = ValidatedWords::<RowPatch>::try_new(patch_words.as_slice()) {
            tx.apply_row_patch(id, RowPatch::view(validated));
        }
    } else {
        tx.attach::<T>(id);
    }
    Some(words)
}

#[inline]
fn sync_str_prop<T: NveTable>(
    tx: &mut TxCore<'_>,
    id: NodeId,
    text_slab: &[u8],
    words: &[u32],
    prop: StrProp<T>,
) {
    let val = read_slab_str(text_slab, words, prop.word());
    tx.set_str(id, prop, val);
}

pub(crate) fn sync_aria_relations(
    tx: &mut TxCore<'_>,
    id: NodeId,
    rid: u32,
    resources: &dyn ResourceReader,
    word_scratch: &mut Vec<u32>,
) {
    if rid == 0 {
        tx.set_resource(id, Accessibility::RELATIONS, None);
        return;
    }
    let Some(bytes) = resources.read(rid, AriaRelations::KIND.0) else {
        tx.set_resource(id, Accessibility::RELATIONS, None);
        return;
    };
    if !bytes.len().is_multiple_of(4) {
        tx.set_resource(id, Accessibility::RELATIONS, None);
        return;
    }
    word_scratch.clear();
    let word_count = bytes.len() / 4;
    word_scratch.reserve(word_count);
    let mut reader = ByteReader::new(bytes);
    for _ in 0..word_count {
        let Ok(word) = reader.read_u32() else {
            tx.set_resource(id, Accessibility::RELATIONS, None);
            return;
        };
        word_scratch.push(word);
    }
    if let Some(validated) = ValidatedWords::<AriaRelations>::try_new(word_scratch.as_slice()) {
        let owned = AriaRelations::view(validated).to_owned();
        tx.set_resource(id, Accessibility::RELATIONS, Some(owned));
    } else {
        tx.set_resource(id, Accessibility::RELATIONS, None);
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn sync_all_tables(
    tx: &mut TxCore<'_>,
    id: NodeId,
    node_idx: usize,
    row: &NveNodeRow,
    text_slab: &[u8],
    bound: &BoundA11yTables<'_>,
    resources: &dyn ResourceReader,
    patch_words: &mut Vec<u32>,
    res_words: &mut Vec<u32>,
) -> u32 {
    let _ = sync_table_scalars::<Layout>(tx, id, node_idx, row, bound.layout, patch_words);
    let _ = sync_table_scalars::<Visual>(tx, id, node_idx, row, bound.visual, patch_words);
    let _ = sync_table_scalars::<Transform>(tx, id, node_idx, row, bound.transform, patch_words);
    let _ = sync_table_scalars::<Scroll>(tx, id, node_idx, row, bound.scroll, patch_words);
    let _ =
        sync_table_scalars::<Interaction>(tx, id, node_idx, row, bound.interaction, patch_words);

    let mut aria_rel_rid = 0u32;
    if let Some(w) =
        sync_table_scalars::<Accessibility>(tx, id, node_idx, row, bound.accessibility, patch_words)
    {
        sync_str_prop(tx, id, text_slab, w, Accessibility::LABEL);
        sync_str_prop(tx, id, text_slab, w, Accessibility::HINT);
        sync_str_prop(tx, id, text_slab, w, Accessibility::VALUE_TEXT);
        let rid = w
            .get(Accessibility::RELATIONS.word() as usize)
            .copied()
            .unwrap_or(0);
        aria_rel_rid = rid;
        sync_aria_relations(tx, id, rid, resources, res_words);
    }
    if let Some(w) =
        sync_table_scalars::<DebugId>(tx, id, node_idx, row, bound.debug_id, patch_words)
    {
        sync_str_prop(tx, id, text_slab, w, DebugId::ID);
        sync_str_prop(tx, id, text_slab, w, DebugId::TEST_ID);
        sync_str_prop(tx, id, text_slab, w, DebugId::SLOT);
    }
    if let Some(w) =
        sync_table_scalars::<TextContent>(tx, id, node_idx, row, bound.text_content, patch_words)
    {
        sync_str_prop(tx, id, text_slab, w, TextContent::TEXT);
    }
    if let Some(w) =
        sync_table_scalars::<TextInput>(tx, id, node_idx, row, bound.text_input, patch_words)
    {
        sync_str_prop(tx, id, text_slab, w, TextInput::VALUE);
        sync_str_prop(tx, id, text_slab, w, TextInput::PLACEHOLDER);
        sync_str_prop(tx, id, text_slab, w, TextInput::PATTERN);
    }
    if let Some(w) = sync_table_scalars::<Link>(tx, id, node_idx, row, bound.link, patch_words) {
        sync_str_prop(tx, id, text_slab, w, Link::HREF);
    }
    if let Some(w) = sync_table_scalars::<Image>(tx, id, node_idx, row, bound.image, patch_words) {
        sync_str_prop(tx, id, text_slab, w, Image::ALT);
    }
    aria_rel_rid
}
