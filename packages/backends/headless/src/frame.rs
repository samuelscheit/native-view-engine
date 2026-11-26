//! Direct zero-overhead [`RenderSink`] and binary [`RenderBackend`] frame submission for
//! [`crate::HeadlessBackend`].

use std::sync::Arc;

use nve_abi::{
    ComponentKind, FrameId, LayoutProps, NativeEvent, NodeId, RawPropValueRef, StyleProps,
    SurfaceId, ROOT_NODE_ID,
};
use nve_components::canvas_ir::DrawList;
use nve_components::interaction::{CursorKind, StateStyleRule};
use nve_components::layout_ext::LayoutExt;
use nve_components::native_view::TypedNativeViewProps;
use nve_components::paint::BackgroundFill;
use nve_components::svg::SvgPicture;
use nve_components::transform::Transform2D;
use nve_engine::{
    replay_command_buffer_into, BackendApplyStats, BackendError, LayoutEngine, LayoutRect,
    RenderBackend, RenderSink, Viewport,
};

use crate::validate::{is_truthy_raw_prop_ref, validate_layout, validate_style};
use crate::{HeadlessBackend, HeadlessMutation};

impl<L: LayoutEngine> RenderSink for HeadlessBackend<L> {
    fn begin_frame(&mut self, surface_id: SurfaceId, _frame_id: FrameId) {
        self.surface_id = surface_id;
        self.last_mutations.clear();
        self.frame_error = None;
        self.frame_op_count = 0;
    }

    fn create_node(&mut self, node_id: NodeId, kind: ComponentKind, custom_name: Option<&str>) {
        if self.frame_error.is_some() {
            return;
        }
        self.frame_op_count = self.frame_op_count.saturating_add(1);
        self.ensure_node(node_id, kind, custom_name);
        if self.track_mutations {
            self.last_mutations
                .push(HeadlessMutation::Create(node_id, kind));
        }
    }

    fn remove_node(&mut self, node_id: NodeId) {
        if self.frame_error.is_some() {
            return;
        }
        self.frame_op_count = self.frame_op_count.saturating_add(1);
        if node_id == ROOT_NODE_ID {
            self.frame_error = Some(BackendError::InvalidNode(node_id));
            return;
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
        self.scroll_offsets.remove(&node_id);
        let _ = self.interaction.remove_node(node_id);
        if self.track_mutations {
            self.last_mutations.push(HeadlessMutation::Remove(node_id));
        }
    }

    fn set_children(&mut self, parent_id: NodeId, children: &[NodeId]) {
        if self.frame_error.is_some() {
            return;
        }
        self.frame_op_count = self.frame_op_count.saturating_add(1);
        let Some((parent_kind, parent_parent)) =
            self.nodes.get(parent_id).map(|n| (n.kind, n.parent))
        else {
            self.frame_error = Some(BackendError::InvalidNode(parent_id));
            return;
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
        for &cid in children {
            if cid == parent_id
                || cid == ROOT_NODE_ID
                || anc_gen.is_some_and(|g| self.visit_stamps.has(cid, g))
            {
                self.frame_error = Some(BackendError::Internal(format!(
                    "cyclic or root child {cid} under parent {parent_id}"
                )));
                return;
            }
            if let Some(g) = child_gen
                && !self.visit_stamps.mark(cid, g)
            {
                self.frame_error = Some(BackendError::Internal(format!(
                    "duplicate child {cid} under parent {parent_id}"
                )));
                return;
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

    fn set_text(&mut self, node_id: NodeId, text: &str) {
        if self.frame_error.is_some() {
            return;
        }
        self.frame_op_count = self.frame_op_count.saturating_add(1);
        let Some(node) = self.nodes.get_mut(node_id) else {
            self.frame_error = Some(BackendError::InvalidNode(node_id));
            return;
        };
        if node.text.as_deref() != Some(text) {
            node.text = Some(Arc::from(text));
        }
        if self.track_mutations {
            self.last_mutations
                .push(HeadlessMutation::UpdateText(node_id, text.to_owned()));
        }
        self.layout.set_text(node_id, text);
    }

    fn set_layout(&mut self, node_id: NodeId, layout: &LayoutProps) {
        if self.frame_error.is_some() {
            return;
        }
        self.frame_op_count = self.frame_op_count.saturating_add(1);
        if let Err(err) = validate_layout(layout) {
            self.frame_error = Some(err);
            return;
        }
        let Some(node) = self.nodes.get_mut(node_id) else {
            self.frame_error = Some(BackendError::InvalidNode(node_id));
            return;
        };
        node.layout = *layout;
        self.layout.set_layout(node_id, layout);
    }

    fn set_style(&mut self, node_id: NodeId, style: &StyleProps) {
        if self.frame_error.is_some() {
            return;
        }
        self.frame_op_count = self.frame_op_count.saturating_add(1);
        if let Err(err) = validate_style(style) {
            self.frame_error = Some(err);
            return;
        }
        let Some(node) = self.nodes.get_mut(node_id) else {
            self.frame_error = Some(BackendError::InvalidNode(node_id));
            return;
        };
        node.style = *style;
        node.has_explicit_style = true;
        if node.kind == ComponentKind::Text && self.has_nested_text {
            self.text_style_dirty = true;
        }
        if self.track_mutations {
            self.last_mutations
                .push(HeadlessMutation::UpdateStyle(node_id));
        }
        self.layout.set_style(node_id, style);
    }

    fn set_raw_prop(&mut self, node_id: NodeId, key: &str, value: RawPropValueRef<'_>) {
        if self.frame_error.is_some() {
            return;
        }
        self.frame_op_count = self.frame_op_count.saturating_add(1);
        let Some(node) = self.nodes.get_mut(node_id) else {
            self.frame_error = Some(BackendError::InvalidNode(node_id));
            return;
        };
        if matches!(key, "flex-wrap" | "flexWrap" | "font-weight" | "fontWeight") {
            match value {
                RawPropValueRef::String(s) => {
                    self.layout.set_raw_layout_prop(node_id, key, s);
                }
                RawPropValueRef::I64(n) => {
                    let s = n.to_string();
                    self.layout.set_raw_layout_prop(node_id, key, &s);
                }
                RawPropValueRef::Null => {
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

    fn set_event_mask(&mut self, node_id: NodeId, mask: u64) {
        if self.frame_error.is_some() {
            return;
        }
        self.frame_op_count = self.frame_op_count.saturating_add(1);
        let Some(node) = self.nodes.get_mut(node_id) else {
            self.frame_error = Some(BackendError::InvalidNode(node_id));
            return;
        };
        node.event_mask = mask;
    }

    fn dispatch_command(&mut self, node_id: NodeId, command_name: &str, payload: &[u8]) {
        if self.frame_error.is_some() {
            return;
        }
        self.frame_op_count = self.frame_op_count.saturating_add(1);
        self.dispatched_commands
            .push((node_id, command_name.to_owned(), payload.to_vec()));
    }

    fn set_layout_ext(&mut self, node_id: NodeId, ext: Option<&LayoutExt>) {
        if self.frame_error.is_some() {
            return;
        }
        let _ = self.set_node_layout_ext(node_id, ext.cloned());
    }

    fn set_transform(&mut self, node_id: NodeId, transform: Option<Transform2D>) {
        if self.frame_error.is_some() {
            return;
        }
        let _ = self.set_node_transform(node_id, transform);
    }

    fn set_cursor(&mut self, node_id: NodeId, cursor: Option<CursorKind>) {
        if self.frame_error.is_some() {
            return;
        }
        let _ = self.set_node_cursor(node_id, cursor);
    }

    fn set_state_rules(&mut self, node_id: NodeId, rules: &[StateStyleRule]) {
        if self.frame_error.is_some() {
            return;
        }
        let _ = self.set_node_state_rules(node_id, rules.to_vec());
    }

    fn set_background_fill(&mut self, node_id: NodeId, fill: Option<&BackgroundFill>) {
        if self.frame_error.is_some() {
            return;
        }
        let _ = self.set_node_background_fill(node_id, fill.cloned());
    }

    fn set_svg(&mut self, node_id: NodeId, svg: Option<&SvgPicture>) {
        if self.frame_error.is_some() {
            return;
        }
        let _ = self.set_node_svg(node_id, svg.cloned());
    }

    fn set_draw_list(&mut self, node_id: NodeId, draw_list: Option<&DrawList>) {
        if self.frame_error.is_some() {
            return;
        }
        let _ = self.set_node_draw_list(node_id, draw_list.cloned());
    }

    fn set_native_view_props(&mut self, node_id: NodeId, props: Option<&TypedNativeViewProps>) {
        if self.frame_error.is_some() {
            return;
        }
        let _ = self.set_native_view_props(node_id, props.cloned());
    }

    fn commit_frame(&mut self, _sync_mount: bool) -> Result<BackendApplyStats, BackendError> {
        if let Some(err) = self.frame_error.take() {
            return Err(err);
        }
        let (layout_us, diff_us) = self.run_layout_and_diff(true);
        let mutation_count = self.last_mutations.len() as u32;
        Ok(BackendApplyStats {
            op_count: self.frame_op_count.saturating_add(1),
            mutation_count,
            decode_us: 0,
            layout_us,
            diff_us,
            mount_us: 0,
        })
    }

    fn finish_uncommitted(&mut self) -> Result<BackendApplyStats, BackendError> {
        if let Some(err) = self.frame_error.take() {
            return Err(err);
        }
        let mutation_count = self.last_mutations.len() as u32;
        Ok(BackendApplyStats {
            op_count: self.frame_op_count,
            mutation_count,
            decode_us: 0,
            layout_us: 0,
            diff_us: 0,
            mount_us: 0,
        })
    }
}

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
        replay_command_buffer_into(command_bytes, self)
    }

    fn drain_events(&mut self) -> Vec<NativeEvent> {
        std::mem::take(&mut self.pending_events)
    }

    #[inline]
    fn layout_of(&self, node_id: NodeId) -> Option<LayoutRect> {
        self.nodes.get(node_id).map(|n| n.computed_rect)
    }
}
