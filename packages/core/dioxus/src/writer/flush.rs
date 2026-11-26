//! Frame flush, dirty-set coalescing, and inherited `Text` style propagation for `NveMutationWriter`.

use nve_abi::{ComponentKind, NodeId, RawPropValue};

use super::{NveMutationWriter, TextStyleWalkEntry};

impl NveMutationWriter {
    pub fn finish_frame(&mut self, sync_mount: bool) -> &[u8] {
        // Ensure any `<NativeView>` node whose `__nve_native_kind` attribute was omitted still
        // emits `CreateNode` before layout/style/children ops.
        if !self.deferred_native_nodes.is_empty() {
            let pending = std::mem::take(&mut self.deferred_native_nodes);
            for nid in pending {
                self.ensure_native_node_created(nid, None);
            }
        }
        // Propagate inherited text_color, font_size, font_weight, and text_align from styled Text
        // parents to unstyled child Text leaves (Dioxus rsx! emits `text { nve_style: ..., "leaf" }`
        // or `text { style: "...", "leaf" }` as parent Text -> child Text).
        if !self.dirty_children.is_empty()
            || !self.dirty_style.is_empty()
            || !self.dirty_text_props.is_empty()
        {
            self.style_sources.clear();
            self.style_sources.extend_from_slice(&self.dirty_children);
            self.style_sources.extend_from_slice(&self.dirty_style);
            self.style_sources.extend_from_slice(&self.dirty_text_props);
            self.style_sources.sort_unstable();
            self.style_sources.dedup();
            for i in 0..self.style_sources.len() {
                let pid = self.style_sources[i];
                self.propagate_text_style_down(pid);
            }
            self.style_sources.clear();
            self.dirty_text_props.clear();
        }

        // Coalesce all structural/style/layout mutations into one deterministic batch per frame
        // using persistent scratch Vecs sorted & deduped in place (zero per-frame heap allocs).
        if !self.dirty_children.is_empty() {
            self.dirty_children.sort_unstable();
            self.dirty_children.dedup();
            for &pid in &self.dirty_children {
                if let Some(rec) = self.nodes.get(&pid) {
                    self.builder.set_children(pid, &rec.children);
                }
            }
            self.dirty_children.clear();
        }

        if !self.dirty_layout.is_empty() {
            self.dirty_layout.sort_unstable();
            self.dirty_layout.dedup();
            for &nid in &self.dirty_layout {
                if let Some(rec) = self.nodes.get(&nid) {
                    self.builder.set_layout(nid, &rec.layout);
                }
            }
            self.dirty_layout.clear();
        }

        if !self.dirty_style.is_empty() {
            self.dirty_style.sort_unstable();
            self.dirty_style.dedup();
            for &nid in &self.dirty_style {
                if let Some(rec) = self.nodes.get(&nid) {
                    self.builder.set_style(nid, &rec.style);
                }
            }
            self.dirty_style.clear();
        }

        if !self.dirty_events.is_empty() {
            self.dirty_events.sort_unstable();
            self.dirty_events.dedup();
            for &nid in &self.dirty_events {
                if let Some(rec) = self.nodes.get(&nid) {
                    self.builder.set_event_mask(nid, rec.event_mask);
                }
            }
            self.dirty_events.clear();
        }

        self.builder.commit(sync_mount);
        self.builder.finish()
    }

    fn propagate_text_style_down(&mut self, root_parent_id: NodeId) {
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
                if !child.has_explicit_style()
                    && (child.style.text_color != entry.color
                        || (child.style.font_size - entry.font_size).abs() > f32::EPSILON)
                {
                    child.style.text_color = entry.color;
                    child.style.font_size = entry.font_size;
                    self.dirty_style.push(cid);
                }
                if !child.has_explicit_font_weight()
                    && child.extras().and_then(|e| e.font_weight.as_deref())
                        != entry.font_weight.as_deref()
                {
                    match &entry.font_weight {
                        Some(w) => self.builder.set_raw_prop_str(cid, "font-weight", w),
                        None => self
                            .builder
                            .set_raw_prop(cid, "font-weight", &RawPropValue::Null),
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
                        Some(a) => self.builder.set_raw_prop_str(cid, "text-align", a),
                        None => self
                            .builder
                            .set_raw_prop(cid, "text-align", &RawPropValue::Null),
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
}
