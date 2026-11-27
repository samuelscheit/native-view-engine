//! Frame flush, dirty-set coalescing, extended typed property flushing, and inherited `Text`
//! style propagation for [`MutationTx`] / [`NveMutationWriter`].

use nve_abi::{ComponentKind, NodeId, RawPropValueRef};
use nve_engine::{BackendApplyStats, BackendError, RenderSink};

use super::{
    MutationTx, NveMutationWriter, TextStyleWalkEntry, EXT_BACKGROUND_FILL, EXT_CURSOR,
    EXT_DRAW_LIST, EXT_LAYOUT_EXT, EXT_NATIVE_VIEW_PROPS, EXT_RESPONSIVE_RULES, EXT_SHOW_ON,
    EXT_STATE_RULES, EXT_SVG, EXT_TRANSFORM, EXT_TRANSITIONS,
};

impl<S: RenderSink> MutationTx<'_, S> {
    /// Flushes all coalesced structural, layout, style, event, and extended typed mutations
    /// directly into `self.sink` and commits the frame.
    pub fn finish_frame(self, sync_mount: bool) -> Result<BackendApplyStats, BackendError> {
        let w = self.writer;
        let sink = self.sink;

        // Ensure any `<NativeView>` node whose `__nve_native_kind` attribute was omitted still
        // emits `CreateNode` before layout/style/children/extended ops.
        if !w.deferred_native_nodes.is_empty() {
            let pending = std::mem::take(&mut w.deferred_native_nodes);
            for nid in pending {
                w.ensure_native_node_created(sink, nid, None);
            }
        }

        // Compose layered styles (`nve_style` -> standalone attrs -> inline `style`) ONCE per
        // dirty node per frame (eliminating O(k^2) reparsing and mid-frame raw-prop churn).
        if !w.dirty_compose.is_empty() {
            let dirty = std::mem::take(&mut w.dirty_compose);
            for &nid in &dirty {
                w.recompute_node_styles(sink, nid);
            }
            let mut reusable = dirty;
            reusable.clear();
            w.dirty_compose = reusable;
        }

        // Propagate inherited text_color, font_size, font_weight, and text_align from styled Text
        // parents to unstyled child Text leaves.
        if !w.dirty_children.is_empty()
            || !w.dirty_style.is_empty()
            || !w.dirty_text_props.is_empty()
        {
            w.style_sources.clear();
            w.style_sources.extend_from_slice(&w.dirty_children);
            w.style_sources.extend_from_slice(&w.dirty_style);
            w.style_sources.extend_from_slice(&w.dirty_text_props);
            w.style_sources.sort_unstable();
            w.style_sources.dedup();
            for i in 0..w.style_sources.len() {
                let pid = w.style_sources[i];
                w.propagate_text_style_down(sink, pid);
            }
            w.style_sources.clear();
            w.dirty_text_props.clear();
        }

        // Resolve symbolic `currentColor` borders in O(k * depth) ONLY for live `currentColor`
        // border nodes and ONLY when a color or structural change occurred this frame.
        if (w.color_tree_dirty || !w.dirty_children.is_empty())
            && !w.current_color_borders.is_empty()
        {
            w.resolve_current_color_borders();
        }
        w.color_tree_dirty = false;

        // Coalesce all structural/style/layout mutations into one deterministic batch per frame
        // using persistent scratch Vecs sorted & deduped in place (zero per-frame heap allocs).
        if !w.dirty_children.is_empty() {
            w.dirty_children.sort_unstable();
            w.dirty_children.dedup();
            for &pid in &w.dirty_children {
                if let Some(rec) = w.nodes.get(&pid) {
                    sink.set_children(pid, &rec.children);
                }
            }
            w.dirty_children.clear();
        }

        if !w.dirty_layout.is_empty() {
            w.dirty_layout.sort_unstable();
            w.dirty_layout.dedup();
            for &nid in &w.dirty_layout {
                if let Some(rec) = w.nodes.get(&nid) {
                    sink.set_layout(nid, &rec.layout);
                }
            }
            w.dirty_layout.clear();
        }

        if !w.dirty_style.is_empty() {
            w.dirty_style.sort_unstable();
            w.dirty_style.dedup();
            for &nid in &w.dirty_style {
                if let Some(rec) = w.nodes.get(&nid) {
                    sink.set_style(nid, &rec.style);
                }
            }
            w.dirty_style.clear();
        }

        if !w.dirty_events.is_empty() {
            w.dirty_events.sort_unstable();
            w.dirty_events.dedup();
            for &nid in &w.dirty_events {
                if let Some(rec) = w.nodes.get(&nid) {
                    sink.set_event_mask(nid, rec.event_mask);
                }
            }
            w.dirty_events.clear();
        }

        if !w.dirty_extended.is_empty() {
            if w.dirty_extended.len() > 1 {
                w.dirty_extended.sort_unstable();
                w.dirty_extended.dedup();
            }
            for &nid in &w.dirty_extended {
                let Some(rec) = w.nodes.get_mut(&nid) else {
                    continue;
                };
                let Some(ext) = rec.extras.as_deref_mut() else {
                    continue;
                };
                let mask = std::mem::take(&mut ext.dirty_ext_mask);
                if mask == 0 {
                    continue;
                }
                if (mask & EXT_LAYOUT_EXT) != 0 {
                    sink.set_layout_ext(nid, ext.layout_ext.as_ref());
                }
                if (mask & EXT_TRANSFORM) != 0 {
                    sink.set_transform(nid, ext.transform);
                }
                if (mask & EXT_CURSOR) != 0 {
                    sink.set_cursor(nid, ext.cursor);
                }
                if (mask & EXT_STATE_RULES) != 0 {
                    sink.set_state_rules(nid, &ext.state_rules);
                }
                if (mask & EXT_TRANSITIONS) != 0 {
                    sink.set_transitions(nid, &ext.transitions);
                }
                if (mask & EXT_SHOW_ON) != 0 {
                    sink.set_show_on(nid, ext.show_on);
                }
                if (mask & EXT_RESPONSIVE_RULES) != 0 {
                    sink.set_responsive_rules(nid, &ext.responsive_rules);
                }
                if (mask & EXT_BACKGROUND_FILL) != 0 {
                    sink.set_background_fill(nid, ext.background_fill.as_ref());
                }
                if (mask & EXT_SVG) != 0 {
                    sink.set_svg(nid, ext.svg_picture.as_ref());
                }
                if (mask & EXT_DRAW_LIST) != 0 {
                    sink.set_draw_list(nid, ext.draw_list.as_ref());
                }
                if (mask & EXT_NATIVE_VIEW_PROPS) != 0 {
                    sink.set_native_view_props(nid, ext.native_props.as_ref());
                }
            }
            w.dirty_extended.clear();
        }

        sink.commit_frame(sync_mount)
    }
}

impl NveMutationWriter {
    fn propagate_text_style_down<S: RenderSink>(&mut self, sink: &mut S, root_parent_id: NodeId) {
        self.style_walk_stack.clear();
        let Some(root) = self.nodes.get(&root_parent_id) else {
            return;
        };
        if root.kind != ComponentKind::Text || root.children.is_empty() {
            return;
        }
        let root_color = root.style.text_color;
        let root_font_size = root.style.font_size;
        let root_font_weight = root.extras().and_then(|e| e.font_weight.clone());
        let root_text_align = root.extras().and_then(|e| e.text_align.clone());
        for &cid in &root.children {
            self.style_walk_stack.push(TextStyleWalkEntry {
                node_id: cid,
                color: root_color,
                font_size: root_font_size,
                font_weight: root_font_weight.clone(),
                text_align: root_text_align.clone(),
            });
        }

        while let Some(entry) = self.style_walk_stack.pop() {
            let cid = entry.node_id;
            if let Some(child) = self.nodes.get_mut(&cid)
                && child.kind == ComponentKind::Text
            {
                let mut style_changed = false;
                if !child.has_explicit_text_color() && child.style.text_color != entry.color {
                    child.style.text_color = entry.color;
                    style_changed = true;
                }
                if !child.has_explicit_font_size()
                    && (child.style.font_size - entry.font_size).abs() > f32::EPSILON
                {
                    child.style.font_size = entry.font_size;
                    style_changed = true;
                }
                if style_changed {
                    self.dirty_style.push(cid);
                }
                if !child.has_explicit_font_weight()
                    && child.extras().and_then(|e| e.font_weight.as_deref())
                        != entry.font_weight.as_deref()
                {
                    match &entry.font_weight {
                        Some(w) => sink.set_raw_prop(cid, "font-weight", RawPropValueRef::String(w)),
                        None => sink.set_raw_prop(cid, "font-weight", RawPropValueRef::Null),
                    }
                    if entry.font_weight.is_some() || child.extras.is_some() {
                        child.extras_mut().font_weight = entry.font_weight.clone();
                    }
                }
                if !child.has_explicit_text_align()
                    && child.extras().and_then(|e| e.text_align.as_deref())
                        != entry.text_align.as_deref()
                {
                    match &entry.text_align {
                        Some(a) => sink.set_raw_prop(cid, "text-align", RawPropValueRef::String(a)),
                        None => sink.set_raw_prop(cid, "text-align", RawPropValueRef::Null),
                    }
                    if entry.text_align.is_some() || child.extras.is_some() {
                        child.extras_mut().text_align = entry.text_align.clone();
                    }
                }
                let next_color = child.style.text_color;
                let next_font_size = child.style.font_size;
                let next_weight = child.extras().and_then(|e| e.font_weight.clone());
                let next_align = child.extras().and_then(|e| e.text_align.clone());
                for &grandchild in &child.children {
                    self.style_walk_stack.push(TextStyleWalkEntry {
                        node_id: grandchild,
                        color: next_color,
                        font_size: next_font_size,
                        font_weight: next_weight.clone(),
                        text_align: next_align.clone(),
                    });
                }
            }
        }
    }

    /// Resolves symbolic `currentColor` borders in O(k * depth) across only the live nodes that
    /// currently use `currentColor`, never walking unrelated subtrees.
    fn resolve_current_color_borders(&mut self) {
        for i in 0..self.current_color_borders.len() {
            let nid = self.current_color_borders[i];
            let Some(rec) = self.nodes.get(&nid) else {
                continue;
            };
            let resolved = if rec.has_explicit_text_color() {
                rec.style.text_color
            } else {
                self.inherited_text_color_from_ancestors(nid)
            };
            if let Some(rec) = self.nodes.get_mut(&nid)
                && rec.style.border_color != resolved
            {
                rec.style.border_color = resolved;
                self.dirty_style.push(nid);
            }
        }
    }
}
