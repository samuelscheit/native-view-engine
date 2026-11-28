//! Cached scene geometry (`rects`, `scroll_offsets`, `scroll_content`, focus, scale) implementing
//! [`A11yGeometryProvider`] for [`crate::WinA11yBridge`].

use nve_abi::{NveLayoutRect, NveScrollOffset};
use nve_accessibility::A11yGeometryProvider;
use nve_engine::LayoutRect;
use nve_store::{NodeId, NodeStore, NveTable};
use nve_tables::tables::Scroll;

pub(crate) struct SceneGeo {
    pub(crate) rects: Vec<LayoutRect>,
    pub(crate) scroll_offsets: Vec<NveScrollOffset>,
    pub(crate) scroll_content: Vec<(u32, f32, f32)>,
    pub(crate) focused_node: Option<NodeId>,
    pub(crate) scale_factor: f32,
}

impl Default for SceneGeo {
    fn default() -> Self {
        Self {
            rects: Vec::new(),
            scroll_offsets: Vec::new(),
            scroll_content: Vec::new(),
            focused_node: None,
            scale_factor: 1.0,
        }
    }
}

impl SceneGeo {
    #[inline]
    pub(crate) fn copy_rect(src: &NveLayoutRect) -> LayoutRect {
        LayoutRect {
            x: src.x,
            y: src.y,
            width: src.width,
            height: src.height,
        }
    }

    pub(crate) fn sync_rects_full(&mut self, rects: &[NveLayoutRect]) {
        self.rects.clear();
        self.rects.reserve(rects.len());
        for r in rects {
            self.rects.push(Self::copy_rect(r));
        }
    }

    /// Resizes `self.rects` to match `rects.len()`, initializing newly appended slots directly
    /// from `rects`.
    pub(crate) fn resize_rects_to(&mut self, rects: &[NveLayoutRect]) {
        if self.rects.len() < rects.len() {
            let old = self.rects.len();
            self.rects.reserve(rects.len() - old);
            for src in &rects[old..] {
                self.rects.push(Self::copy_rect(src));
            }
        } else {
            self.rects.truncate(rects.len());
        }
    }

    /// Updates `self.rects[idx]` from `rects[idx]` and returns `true` when the node's `(x, y)`
    /// absolute origin moved (meaning its descendants' absolute rects may also have shifted).
    #[inline]
    pub(crate) fn sync_one_rect(&mut self, idx: usize, rects: &[NveLayoutRect]) -> bool {
        let (Some(dst), Some(src)) = (self.rects.get_mut(idx), rects.get(idx)) else {
            return false;
        };
        let next = Self::copy_rect(src);
        let origin_moved = (dst.x - next.x).abs() > 1e-6 || (dst.y - next.y).abs() > 1e-6;
        *dst = next;
        origin_moved
    }

    /// Diffs sorted `next` scroll offsets against cached offsets, appending every changed node to
    /// `dirty_out`. Precondition: `next` is sorted ascending by `node_id` (guaranteed by
    /// `nve-runtime`).
    pub(crate) fn sync_scroll_offsets(
        &mut self,
        next: &[NveScrollOffset],
        dirty_out: &mut Vec<NodeId>,
    ) {
        let prev = &self.scroll_offsets;
        let (mut i, mut j) = (0usize, 0usize);
        while i < prev.len() && j < next.len() {
            let a = prev[i];
            let b = next[j];
            if a.node_id == b.node_id {
                if (a.x - b.x).abs() > 1e-6 || (a.y - b.y).abs() > 1e-6 {
                    let nid = NodeId::from_raw_or_null(a.node_id);
                    if !nid.is_null() {
                        dirty_out.push(nid);
                    }
                }
                i += 1;
                j += 1;
            } else if a.node_id < b.node_id {
                let nid = NodeId::from_raw_or_null(a.node_id);
                if !nid.is_null() {
                    dirty_out.push(nid);
                }
                i += 1;
            } else {
                let nid = NodeId::from_raw_or_null(b.node_id);
                if !nid.is_null() {
                    dirty_out.push(nid);
                }
                j += 1;
            }
        }
        for a in &prev[i..] {
            let nid = NodeId::from_raw_or_null(a.node_id);
            if !nid.is_null() {
                dirty_out.push(nid);
            }
        }
        for b in &next[j..] {
            let nid = NodeId::from_raw_or_null(b.node_id);
            if !nid.is_null() {
                dirty_out.push(nid);
            }
        }
        self.scroll_offsets.clear();
        self.scroll_offsets.extend_from_slice(next);
    }

    pub(crate) fn rebuild_scroll_content(&mut self, store: &NodeStore) {
        self.scroll_content.clear();
        let Some(storage) = store.table_storage(Scroll::slot()) else {
            return;
        };
        for &owner in storage.owners {
            if !store.is_live(owner) {
                continue;
            }
            let Some(parent_rect) = self.rects.get(owner.index()).copied() else {
                continue;
            };
            let mut max_w = parent_rect.width.max(0.0);
            let mut max_h = parent_rect.height.max(0.0);
            for &child in store.children(owner) {
                if let Some(cr) = self.rects.get(child.index()) {
                    max_w = max_w.max((cr.x + cr.width) - parent_rect.x);
                    max_h = max_h.max((cr.y + cr.height) - parent_rect.y);
                }
            }
            self.scroll_content.push((owner.raw(), max_w, max_h));
        }
        self.scroll_content.sort_unstable_by_key(|&(nid, _, _)| nid);
    }
}

impl A11yGeometryProvider for SceneGeo {
    #[inline]
    fn scale_factor(&self) -> f32 {
        self.scale_factor
    }

    #[inline]
    fn layout_rect(&self, id: NodeId) -> Option<LayoutRect> {
        self.rects.get(id.index()).copied()
    }

    #[inline]
    fn scroll_offset(&self, id: NodeId) -> (f32, f32) {
        self.scroll_offsets
            .binary_search_by_key(&id.raw(), |s| s.node_id)
            .map_or((0.0, 0.0), |i| {
                (self.scroll_offsets[i].x, self.scroll_offsets[i].y)
            })
    }

    #[inline]
    fn scroll_content_size(&self, id: NodeId) -> Option<(f32, f32)> {
        self.scroll_content
            .binary_search_by_key(&id.raw(), |&(nid, _, _)| nid)
            .ok()
            .map(|i| (self.scroll_content[i].1, self.scroll_content[i].2))
    }

    #[inline]
    fn focused_node(&self) -> Option<NodeId> {
        self.focused_node
    }
}
