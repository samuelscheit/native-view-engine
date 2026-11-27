//! Per-frame `NveSceneTable` mirror of a live `NodeStore` tree (feature `scene-view`), shared by
//! [`crate::WebAppSession`] and [`crate::PairedWebSession`].

use nve_abi::table::NveTableView;
use nve_abi::{
    FrameId, NodeId, NveRenderNode, NveSceneRef, NveSceneTable, NveSceneView, PrimarySlot,
    SceneNodeInput, ShortLen, ZIndex, NVE_NODE_CLIP_BOUNDS, NVE_NODE_INTERACTIVE,
    NVE_NODE_LEAF_TEXT,
};
use nve_components::tables::{Interaction, Layout, Overflow, TextContent, Typography, Visual};
use nve_store::{ChangeSet, NodeId as StoreNodeId, NodeStore, NveTable};

fn store_node_to_scene_input<'a>(
    store: &'a NodeStore,
    node_id: StoreNodeId,
    children_scratch: &'a mut Vec<NodeId>,
) -> (SceneNodeInput<'a>, u32, u16) {
    let children = store.children(node_id);
    children_scratch.clear();
    children_scratch.extend(children.iter().copied());

    let (text_offset, text_len) = if let Some(tc) = store.try_row::<TextContent>(node_id) {
        let span = tc.text_span();
        let s = tc.text();
        let clamped_len = if s.len() <= u16::MAX as usize {
            s.len() as u16
        } else {
            let mut end = u16::MAX as usize;
            while !s.is_char_boundary(end) {
                end -= 1;
            }
            end as u16
        };
        if clamped_len == 0 {
            (0u32, 0u16)
        } else {
            (span.off(), clamped_len)
        }
    } else {
        (0u32, 0u16)
    };

    let mut flags = 0u8;
    if store.has::<TextContent>(node_id) && children.is_empty() {
        flags |= NVE_NODE_LEAF_TEXT;
    }
    if store
        .try_row::<Interaction>(node_id)
        .is_some_and(|i| i.event_mask() != 0)
    {
        flags |= NVE_NODE_INTERACTIVE;
    }
    if store.try_row::<Visual>(node_id).is_some_and(|v| {
        matches!(v.overflow_x(), Overflow::Hidden | Overflow::Scroll)
            || matches!(v.overflow_y(), Overflow::Hidden | Overflow::Scroll)
    }) {
        flags |= NVE_NODE_CLIP_BOUNDS;
    }

    let (width, height) = store.try_row::<Layout>(node_id).map_or((0.0, 0.0), |l| {
        let w = if !l.width().is_auto() && !l.width().is_percent() {
            l.width().value()
        } else {
            0.0
        };
        let h = if !l.height().is_auto() && !l.height().is_percent() {
            l.height().value()
        } else {
            0.0
        };
        (w, h)
    });

    let (opacity_u8, bg_rgba, border_rgba, border_radius, border_width, z_index) = store
        .try_row::<Visual>(node_id)
        .map_or((255, 0, 0, 0.0, 0.0, 0), |v| {
            (
                NveRenderNode::encode_opacity_u8(v.opacity()),
                v.background().0,
                v.border_color().0,
                v.border_radius(),
                v.border_width(),
                v.z_index(),
            )
        });

    let (text_rgba, font_size) = store
        .try_row::<Typography>(node_id)
        .map_or((0, 16.0), |t| (t.color().0, t.font_size()));

    (
        SceneNodeInput {
            node_id,
            primary_slot: PrimarySlot(store.primary_slot(node_id).0),
            flags,
            opacity_u8,
            x: 0.0,
            y: 0.0,
            width,
            height,
            bg_rgba,
            text_rgba,
            border_rgba,
            border_radius,
            border_width,
            font_size,
            z_index: ZIndex(z_index),
            text: "",
            children: children_scratch.as_slice(),
        },
        text_offset,
        text_len,
    )
}

/// Per-frame `NveSceneTable` mirror of a live `NodeStore` behind a stable `NveSceneView`
/// header in linear memory (`nve_web_scene_view_ptr`, zero-copy JS inspection).
pub struct SceneMirror {
    table: NveSceneTable,
    view: Box<NveSceneView>,
    sort_scratch: Vec<StoreNodeId>,
    children_scratch: Vec<NodeId>,
    text_dirty_marks: Vec<u64>,
    text_dirty_epoch: u64,
    published_tables: Vec<NveTableView>,
    published_dirty_buf: Vec<u32>,
}

impl SceneMirror {
    pub(crate) fn new() -> Self {
        let table = NveSceneTable::new();
        let view = Box::new(table.as_view());
        Self {
            table,
            view,
            sort_scratch: Vec::new(),
            children_scratch: Vec::new(),
            text_dirty_marks: Vec::new(),
            text_dirty_epoch: 0,
            published_tables: Vec::new(),
            published_dirty_buf: Vec::new(),
        }
    }

    /// The incremental [`NveSceneTable`] mirroring the live `NodeStore` tree.
    pub fn table(&self) -> &NveSceneTable {
        &self.table
    }

    /// A zero-copy C ABI [`NveSceneView`] pointing directly into the scene slabs.
    pub fn view(&self) -> NveSceneView {
        *self.view
    }

    /// Stable pointer to the [`NveSceneView`] header in linear memory (suitable for
    /// `NveSceneViewReader.fromWasm32Pointer(memory.buffer, ptr)` on `wasm32`).
    pub fn view_ptr(&self) -> *const NveSceneView {
        &*self.view as *const NveSceneView
    }

    /// A safe zero-copy borrowed [`NveSceneRef`] paired with `store`'s unified UTF-8 `text_slab`.
    pub fn as_ref_with_store<'a>(&'a self, store: &'a NodeStore) -> NveSceneRef<'a> {
        self.table.as_ref_with_external_text(store.text_slab())
    }

    pub(crate) fn sync(&mut self, store: &NodeStore, changes: &ChangeSet, frame_id: FrameId) {
        self.sort_scratch.clear();
        self.sort_scratch.extend(store.live_nodes());
        self.sort_scratch.sort_unstable_by_key(|id| id.raw());

        self.table.begin_frame(frame_id);
        let has_text_changes = if let Some(td) = changes.table(TextContent::slot()) {
            self.text_dirty_epoch = self.text_dirty_epoch.wrapping_add(1);
            if self.text_dirty_epoch == 0 {
                self.text_dirty_marks.fill(0);
                self.text_dirty_epoch = 1;
            }
            if self.text_dirty_marks.len() < store.capacity() {
                self.text_dirty_marks.resize(store.capacity(), 0);
            }
            let epoch = self.text_dirty_epoch;
            for &nid in td.nodes.iter().chain(td.detached.iter()) {
                let idx = nid.index();
                if idx < self.text_dirty_marks.len() {
                    self.text_dirty_marks[idx] = epoch;
                }
            }
            true
        } else {
            false
        };

        for &node_id in &self.sort_scratch {
            let text_dirty = has_text_changes
                && self.text_dirty_marks.get(node_id.index()).copied()
                    == Some(self.text_dirty_epoch);
            let (input, text_offset, text_len) =
                store_node_to_scene_input(store, node_id, &mut self.children_scratch);
            let _ = self.table.upsert_node_external_text(
                input,
                text_offset,
                ShortLen(text_len),
                text_dirty,
            );
        }
        self.table.prune_unseen();
        self.table.maybe_compact_slabs();

        store.publish_table_views(
            changes,
            &mut self.published_tables,
            &mut self.published_dirty_buf,
        );
        let mut view = self.table.as_view_with_external_text(store.text_slab());
        view.table_count = self.published_tables.len() as u32;
        view.tables = if self.published_tables.is_empty() {
            core::ptr::null()
        } else {
            self.published_tables.as_ptr()
        };
        *self.view = view;
    }
}
