//! `RenderBackend` binary `CommandBuffer` frame submission for [`crate::HeadlessBackend`].

use std::sync::Arc;
use std::time::Instant;

use nve_abi::{
    CommandDecoder, CommandRef, ComponentKind, NativeEvent, NodeId, SurfaceId, ROOT_NODE_ID,
};
use nve_engine::{BackendApplyStats, BackendError, LayoutEngine, LayoutRect, RenderBackend, Viewport};

use crate::validate::{is_truthy_raw_prop_ref, validate_layout, validate_style};
use crate::{HeadlessBackend, HeadlessMutation};

impl<L: LayoutEngine> RenderBackend for HeadlessBackend<L> {
    fn name(&self) -> &'static str {
        match self.layout.name() {
            "taffy" => "headless-taffy",
            "yoga" => "headless-yoga",
            _ => "headless-custom",
        }
    }

    fn set_viewport(&mut self, surface_id: SurfaceId, viewport: Viewport) {
        self.surface_id = surface_id;
        self.viewport = viewport;
    }

    fn submit_frame(&mut self, command_bytes: &[u8]) -> Result<BackendApplyStats, BackendError> {
        let t_decode = Instant::now();
        self.last_mutations.clear();
        let track_mutations = self.track_mutations;
        let mut committed = false;

        let decoder = CommandDecoder::new(command_bytes)?;
        if track_mutations {
            let est_ops = (decoder.header().op_count as usize).min(command_bytes.len() / 2);
            self.last_mutations.reserve(est_ops);
        }

        let header = decoder.for_each(|cmd| {
            match cmd {
                CommandRef::CreateNode { node_id, kind, custom_name } => {
                    self.ensure_node(node_id, kind, custom_name);
                    if track_mutations {
                        self.last_mutations
                            .push(HeadlessMutation::Create(node_id, kind));
                    }
                }
                CommandRef::RemoveNode { node_id } => {
                    if node_id == ROOT_NODE_ID {
                        return Err(BackendError::InvalidNode(node_id));
                    }
                    let mut parent_to_resync: Option<NodeId> = None;
                    if let Some(removed) = self.nodes.remove(node_id) {
                        if let Some(pid) = removed.parent
                            && let Some(pnode) = self.nodes.get_mut(pid)
                        {
                            pnode.children.retain(|&c| c != node_id);
                            parent_to_resync = Some(pid);
                        }
                        for cid in removed.children {
                            if let Some(cnode) = self.nodes.get_mut(cid)
                                && cnode.parent == Some(node_id)
                            {
                                cnode.parent = None;
                            }
                        }
                        if self.has_nested_text {
                            self.text_style_dirty = true;
                        }
                    }
                    if let Some(pid) = parent_to_resync
                        && let Some(pnode) = self.nodes.get(pid)
                    {
                        self.layout.set_children(pid, &pnode.children);
                    }
                    self.layout.remove_node(node_id);
                    self.prev_rects.remove(node_id);
                    let _ = self.interaction.remove_node(node_id);
                    if track_mutations {
                        self.last_mutations.push(HeadlessMutation::Remove(node_id));
                    }
                }
                CommandRef::SetChildren { parent_id, children } => {
                    let Some((parent_kind, parent_parent)) =
                        self.nodes.get(parent_id).map(|n| (n.kind, n.parent))
                    else {
                        return Err(BackendError::InvalidNode(parent_id));
                    };
                    let anc_gen = if parent_parent.is_some() {
                        let g = self.visit_stamps.next_gen();
                        let mut walk = parent_parent;
                        for _ in 0..=self.nodes.len() {
                            let Some(cur) = walk else { break };
                            if !self.visit_stamps.mark(cur, g) {
                                break;
                            }
                            walk = self.nodes.get(cur).and_then(|n| n.parent);
                        }
                        Some(g)
                    } else {
                        None
                    };

                    let check_dups = children.len() > 1;
                    let child_gen = check_dups.then(|| self.visit_stamps.next_gen());
                    self.scratch_children.clear();
                    self.scratch_children.reserve(children.len());
                    for cid in children {
                        if cid == parent_id
                            || cid == ROOT_NODE_ID
                            || anc_gen.is_some_and(|g| self.visit_stamps.has(cid, g))
                        {
                            return Err(BackendError::Internal(format!(
                                "cyclic or root child {cid} under parent {parent_id}"
                            )));
                        }
                        if let Some(g) = child_gen
                            && !self.visit_stamps.mark(cid, g)
                        {
                            return Err(BackendError::Internal(format!(
                                "duplicate child {cid} under parent {parent_id}"
                            )));
                        }
                        self.scratch_children.push(cid);
                    }

                    self.scratch_old_children.clear();
                    if let Some(parent) = self.nodes.get_mut(parent_id)
                        && !parent.children.is_empty()
                    {
                        self.scratch_old_children.extend_from_slice(&parent.children);
                    }
                    for &old_cid in &self.scratch_old_children {
                        if let Some(cnode) = self.nodes.get_mut(old_cid)
                            && cnode.parent == Some(parent_id)
                        {
                            cnode.parent = None;
                        }
                    }

                    self.scratch_prev_parents.clear();
                    for &cid in &self.scratch_children {
                        if let Some(cnode) = self.nodes.get_mut(cid) {
                            let prev_parent = cnode.parent;
                            cnode.parent = Some(parent_id);
                            if let Some(prev_pid) = prev_parent
                                && prev_pid != parent_id
                            {
                                if let Some(prev_pnode) = self.nodes.get_mut(prev_pid) {
                                    prev_pnode.children.retain(|&c| c != cid);
                                }
                                self.scratch_prev_parents.push(prev_pid);
                            }
                        }
                    }

                    if let Some(parent) = self.nodes.get_mut(parent_id) {
                        parent.children.clear();
                        parent.children.extend_from_slice(&self.scratch_children);
                    }

                    for &prev_pid in &self.scratch_prev_parents {
                        if let Some(pnode) = self.nodes.get(prev_pid) {
                            self.layout.set_children(prev_pid, &pnode.children);
                        }
                    }
                    self.layout.set_children(parent_id, &self.scratch_children);

                    if (parent_kind == ComponentKind::Text && !self.scratch_children.is_empty())
                        || self.has_nested_text
                    {
                        if parent_kind == ComponentKind::Text && !self.scratch_children.is_empty() {
                            self.has_nested_text = true;
                        }
                        self.text_style_dirty = true;
                    }
                }
                CommandRef::SetText { node_id, text } => {
                    let Some(node) = self.nodes.get_mut(node_id) else {
                        return Err(BackendError::InvalidNode(node_id));
                    };
                    if node.text.as_deref() != Some(text) {
                        node.text = Some(Arc::from(text));
                    }
                    if track_mutations {
                        self.last_mutations
                            .push(HeadlessMutation::UpdateText(node_id, text.to_owned()));
                    }
                    self.layout.set_text(node_id, text);
                }
                CommandRef::SetLayout { node_id, layout } => {
                    validate_layout(&layout)?;
                    let Some(node) = self.nodes.get_mut(node_id) else {
                        return Err(BackendError::InvalidNode(node_id));
                    };
                    node.layout = layout;
                    self.layout.set_layout(node_id, &layout);
                }
                CommandRef::SetStyle { node_id, style } => {
                    validate_style(&style)?;
                    let Some(node) = self.nodes.get_mut(node_id) else {
                        return Err(BackendError::InvalidNode(node_id));
                    };
                    node.style = style;
                    node.has_explicit_style = true;
                    if node.kind == ComponentKind::Text && self.has_nested_text {
                        self.text_style_dirty = true;
                    }
                    if track_mutations {
                        self.last_mutations
                            .push(HeadlessMutation::UpdateStyle(node_id));
                    }
                    self.layout.set_style(node_id, &style);
                }
                CommandRef::SetRawProp { node_id, key, value } => {
                    let Some(node) = self.nodes.get_mut(node_id) else {
                        return Err(BackendError::InvalidNode(node_id));
                    };
                    if matches!(key, "flex-wrap" | "flexWrap" | "font-weight" | "fontWeight") {
                        match value {
                            nve_abi::RawPropValueRef::String(s) => {
                                self.layout.set_raw_layout_prop(node_id, key, s);
                            }
                            nve_abi::RawPropValueRef::I64(n) => {
                                let s = n.to_string();
                                self.layout.set_raw_layout_prop(node_id, key, &s);
                            }
                            nve_abi::RawPropValueRef::Null => {
                                self.layout.set_raw_layout_prop(node_id, key, "");
                            }
                            _ => {}
                        }
                    }
                    if key == "disabled" {
                        let _ = self
                            .interaction
                            .set_disabled(node_id, is_truthy_raw_prop_ref(value));
                    }
                    if let Some(existing) = node.raw_props.get_mut(key) {
                        *existing = value.to_owned();
                    } else {
                        node.raw_props.insert(key.to_owned(), value.to_owned());
                    }
                }
                CommandRef::SetEventMask { node_id, mask } => {
                    let Some(node) = self.nodes.get_mut(node_id) else {
                        return Err(BackendError::InvalidNode(node_id));
                    };
                    node.event_mask = mask;
                }
                CommandRef::DispatchCommand { node_id, command_name, payload } => {
                    self.dispatched_commands
                        .push((node_id, command_name.to_owned(), payload.to_vec()));
                }
                CommandRef::Commit { .. } => {
                    committed = true;
                }
            }
            Ok::<(), BackendError>(())
        })?;

        let decode_us = t_decode.elapsed().as_micros() as u64;
        let (layout_us, diff_us) = if committed {
            self.run_layout_and_diff(true)
        } else {
            (0, 0)
        };

        let mutation_count = self.last_mutations.len() as u32;

        Ok(BackendApplyStats {
            op_count: header.op_count,
            mutation_count,
            decode_us,
            layout_us,
            diff_us,
            mount_us: 0,
        })
    }

    fn drain_events(&mut self) -> Vec<NativeEvent> {
        std::mem::take(&mut self.pending_events)
    }

    #[inline]
    fn layout_of(&self, node_id: NodeId) -> Option<LayoutRect> {
        self.nodes.get(node_id).map(|n| n.computed_rect)
    }
}
