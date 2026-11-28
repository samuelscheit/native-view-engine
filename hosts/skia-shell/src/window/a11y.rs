//! Thin `accesskit_winit` adapter wiring and [`DesktopSession`] action/tree bridge over the
//! platform-agnostic [`nve_accessibility`] capability module.

use std::borrow::Cow;

use accesskit::{Affine, TreeUpdate};
use nve_abi::{EventBatchBuf, NodeId, TimestampUs};
use nve_accessibility::{
    char_index_to_byte_offset, A11yAction, A11yGeometryProvider, AccessKitTreeBridge,
    ResolvedA11yAction,
};
use nve_engine::LayoutRect;
use nve_store::{ChangeSet, NodeStore};
use nve_tables::tables::{Accessibility, Interaction, TextInput};
use winit::window::WindowId;

use super::input::is_focusable;
use super::multi_window::SurfaceWindow;
use super::session::{now_us, DesktopSession, ShellSkiaBackend};
use super::text_edit::{insert_text_at, sanitize_pasted_multiline, sanitize_pasted_single_line};
use super::window_host::DesktopWindowHost;
use super::{DesktopApp, DesktopHostError};

/// Returns `true` when `id` is live, attached to `NodeId::ROOT`, and neither `id` nor any
/// ancestor is effectively `display: none` (`Layout::Display::None`, `StateRules` override,
/// `InputKind::Hidden`) or `Accessibility::hidden` (`aria-hidden`).
pub(crate) fn is_a11y_target_accessible(
    store: &NodeStore,
    backend: &ShellSkiaBackend,
    id: NodeId,
) -> bool {
    let mut cur = id;
    for _ in 0..256 {
        if !store.is_live(cur) {
            return false;
        }
        if cur == NodeId::ROOT {
            return true;
        }
        if backend.is_node_display_none_effective(store, cur) {
            return false;
        }
        if store
            .try_row::<Accessibility>(cur)
            .is_some_and(|a| a.hidden())
        {
            return false;
        }
        let Some(parent) = store.parent(cur) else {
            return false;
        };
        if parent == cur {
            return false;
        }
        cur = parent;
    }
    false
}

/// Zero-copy [`A11yGeometryProvider`] adapter over `(&NodeStore, &ShellSkiaBackend)`.
pub(crate) struct SkiaA11yGeometry<'a> {
    store: &'a NodeStore,
    backend: &'a ShellSkiaBackend,
    scale_factor: f32,
    root_has_layout: bool,
}

impl<'a> SkiaA11yGeometry<'a> {
    #[inline]
    pub(crate) fn new(
        store: &'a NodeStore,
        backend: &'a ShellSkiaBackend,
        scale_factor: f32,
    ) -> Self {
        let root_has_layout = backend.layout_tree().layout_of(NodeId::ROOT).is_some();
        Self {
            store,
            backend,
            scale_factor,
            root_has_layout,
        }
    }
}

impl A11yGeometryProvider for SkiaA11yGeometry<'_> {
    #[inline]
    fn scale_factor(&self) -> f32 {
        self.scale_factor
    }

    #[inline]
    fn layout_rect(&self, id: NodeId) -> Option<LayoutRect> {
        self.backend.layout_tree().layout_of(id)
    }

    #[inline]
    fn scroll_offset(&self, id: NodeId) -> (f32, f32) {
        self.backend.scroll_offset(id)
    }

    #[inline]
    fn visual_scroll_shift(&self, id: NodeId) -> Option<(f32, f32)> {
        Some(self.backend.visual_scroll_shift(self.store, id))
    }

    #[inline]
    fn scroll_content_size(&self, id: NodeId) -> Option<(f32, f32)> {
        self.backend.scroll_content_size(self.store, id)
    }

    #[inline]
    fn sticky_delta(&self, id: NodeId) -> (f32, f32) {
        self.backend.layout_tree().compute_sticky_delta(self.store, id)
    }

    #[inline]
    fn node_affine(&self, id: NodeId) -> Option<Affine> {
        let tf = self.backend.effective_transform(self.store, id)?;
        let r = self.backend.layout_tree().layout_of(id)?;
        let m = tf.to_surface_matrix_2d(r.x, r.y, r.width, r.height);
        Some(Affine::new([
            f64::from(m[0]),
            f64::from(m[1]),
            f64::from(m[2]),
            f64::from(m[3]),
            f64::from(m[4]),
            f64::from(m[5]),
        ]))
    }

    #[inline]
    fn focused_node(&self) -> Option<NodeId> {
        self.backend.focused_node()
    }

    #[inline]
    fn text_input_value(&self, id: NodeId) -> Option<Cow<'_, str>> {
        self.backend.text_input_value(self.store, id).map(Cow::Owned)
    }

    #[inline]
    fn text_selection_bytes(&self, id: NodeId) -> Option<(usize, usize)> {
        let fs = self.backend.focus_state();
        if fs.focused_node == Some(id) {
            Some((fs.selection_anchor, fs.caret_byte_index))
        } else {
            None
        }
    }

    #[inline]
    fn is_display_none(&self, id: NodeId) -> bool {
        if id == NodeId::ROOT {
            return false;
        }
        if self.backend.is_node_display_none_effective(self.store, id) {
            return true;
        }
        if self.backend.is_node_display_contents_effective(self.store, id) {
            return false;
        }
        self.root_has_layout && self.backend.layout_tree().layout_of(id).is_none()
    }

    #[inline]
    fn is_display_contents(&self, id: NodeId) -> bool {
        self.backend.is_node_display_contents_effective(self.store, id)
    }
}

impl<A: DesktopApp> DesktopSession<A> {
    #[inline]
    pub(super) fn mark_a11y_dirty(&mut self, id: NodeId) {
        if !self.a11y_dirty_nodes.contains(&id) {
            self.a11y_dirty_nodes.push(id);
        }
    }

    #[inline]
    pub(crate) fn has_pending_a11y_dirty(&self) -> bool {
        self.a11y_needs_full_sync || !self.a11y_dirty_nodes.is_empty()
    }

    /// Builds a complete AccessKit [`TreeUpdate`] for the current replica store and records the
    /// synchronized wire frame counter.
    pub fn build_a11y_full_tree(
        &mut self,
        bridge: &mut AccessKitTreeBridge,
        window_title: &str,
    ) -> TreeUpdate {
        self.a11y_dirty_nodes.clear();
        self.a11y_needs_full_sync = false;
        let geo = SkiaA11yGeometry::new(
            self.replica.store(),
            self.replica.backend(),
            self.viewport.scale_factor,
        );
        self.a11y_synced_wire_frame = self.replica.last_changes().frame;
        bridge.build_full_tree_update(self.replica.store(), &geo, window_title)
    }

    /// Computes a minimal incremental AccessKit [`TreeUpdate`] (returning `None` with 0 allocs
    /// when `bridge` is inactive or only `COMPOSITE`-tier visuals changed), draining any
    /// locally dirty scroll/text/selection/animation node IDs in O(dirty).
    pub fn sync_a11y_incremental(
        &mut self,
        bridge: &mut AccessKitTreeBridge,
        window_title: &str,
        force_geometry_or_scroll: bool,
    ) -> Option<TreeUpdate> {
        let mut dirty = std::mem::take(&mut self.a11y_dirty_nodes);
        let force = force_geometry_or_scroll || std::mem::take(&mut self.a11y_needs_full_sync);
        if !bridge.is_active() {
            dirty.clear();
            self.a11y_dirty_nodes = dirty;
            return None;
        }
        let empty = ChangeSet::default();
        let last_changes = self.replica.last_changes();
        let has_fresh_wire = last_changes.frame != self.a11y_synced_wire_frame;
        let changes = if has_fresh_wire { last_changes } else { &empty };
        let geo = SkiaA11yGeometry::new(
            self.replica.store(),
            self.replica.backend(),
            self.viewport.scale_factor,
        );
        let update = bridge.sync_changes_with_dirty(
            self.replica.store(),
            changes,
            &geo,
            window_title,
            &dirty,
            force,
        );
        if has_fresh_wire {
            self.a11y_synced_wire_frame = last_changes.frame;
        }
        dirty.clear();
        self.a11y_dirty_nodes = dirty;
        update
    }

    /// Dispatches a translated assistive-technology [`ResolvedA11yAction`] into the app and
    /// backend, returning `Ok(true)` when state or pixels changed.
    pub fn dispatch_a11y_action(
        &mut self,
        resolved: ResolvedA11yAction,
    ) -> Result<bool, DesktopHostError> {
        let target = resolved.target;
        if !self.replica.store().is_live(target) {
            return Ok(false);
        }
        if !matches!(resolved.action, A11yAction::Blur)
            && !is_a11y_target_accessible(self.replica.store(), self.replica.backend(), target)
        {
            return Ok(false);
        }
        let is_disabled = self
            .replica
            .store()
            .try_row::<Interaction>(target)
            .is_some_and(|i| i.disabled());

        match resolved.action {
            A11yAction::Click | A11yAction::Expand | A11yAction::Collapse => {
                if is_disabled {
                    return Ok(false);
                }
                let (cx, cy) = self
                    .replica
                    .backend()
                    .layout_tree()
                    .layout_of(target)
                    .map_or((0.0, 0.0), |r| (r.x + r.width * 0.5, r.y + r.height * 0.5));
                let mut events = EventBatchBuf::with_capacity(1, 0);
                events.push_press(self.surface_id, target, TimestampUs(now_us()), cx, cy);
                self.dispatch_and_redraw(&events)?;
                Ok(true)
            }
            A11yAction::Focus => {
                if is_disabled {
                    return Ok(false);
                }
                self.focus_node(Some(target))
            }
            A11yAction::Blur => {
                if self.replica.backend().focused_node() == Some(target) {
                    self.focus_node(None)
                } else {
                    Ok(false)
                }
            }
            A11yAction::SetValue(raw_val) => {
                if is_disabled {
                    return Ok(false);
                }
                if let Some(ti) = self.replica.store().try_row::<TextInput>(target) {
                    if ti.read_only() {
                        return Ok(false);
                    }
                    let multiline = ti.multiline();
                    if self.replica.backend().focused_node() != Some(target) {
                        let _ = self.focus_node(Some(target))?;
                    }
                    let sanitized = if multiline {
                        sanitize_pasted_multiline(&raw_val)
                    } else {
                        sanitize_pasted_single_line(&raw_val)
                    };
                    let end = sanitized.len();
                    return self.emit_text_change(target, sanitized, end, None);
                }
                let mut events = EventBatchBuf::with_capacity(1, raw_val.len());
                events.push_text_change(self.surface_id, target, TimestampUs(now_us()), &raw_val);
                self.dispatch_and_redraw(&events)?;
                Ok(true)
            }
            A11yAction::SetNumericValue(val) => {
                if is_disabled {
                    return Ok(false);
                }
                let text = format!("{val}");
                let mut events = EventBatchBuf::with_capacity(1, text.len());
                events.push_text_change(self.surface_id, target, TimestampUs(now_us()), &text);
                self.dispatch_and_redraw(&events)?;
                Ok(true)
            }
            A11yAction::ReplaceSelectedText(raw_val) => {
                if is_disabled {
                    return Ok(false);
                }
                let Some(ti) = self.replica.store().try_row::<TextInput>(target) else {
                    return Ok(false);
                };
                if ti.read_only() {
                    return Ok(false);
                }
                let multiline = ti.multiline();
                if self.replica.backend().focused_node() != Some(target) {
                    let _ = self.focus_node(Some(target))?;
                }
                let current = self
                    .replica
                    .backend()
                    .text_input_value(self.replica.store(), target)
                    .unwrap_or_default();
                let fs = self.replica.backend().focus_state();
                let caret = fs.caret_byte_index.min(current.len());
                let selection = fs.selection_range();
                let sanitized = if multiline {
                    sanitize_pasted_multiline(&raw_val)
                } else {
                    sanitize_pasted_single_line(&raw_val)
                };
                let (next_val, next_caret) = insert_text_at(&current, caret, selection, &sanitized);
                self.emit_text_change(target, next_val, next_caret, None)
            }
            A11yAction::SetTextSelection {
                anchor_char,
                focus_char,
            } => {
                if is_disabled || !self.replica.store().has::<TextInput>(target) {
                    return Ok(false);
                }
                if self.replica.backend().focused_node() != Some(target) {
                    let _ = self.focus_node(Some(target))?;
                }
                let current = self
                    .replica
                    .backend()
                    .text_input_value(self.replica.store(), target)
                    .unwrap_or_default();
                let anchor_byte = char_index_to_byte_offset(&current, anchor_char);
                let focus_byte = char_index_to_byte_offset(&current, focus_char);
                {
                    let (store, backend) = self.replica.store_and_backend_mut();
                    backend.set_caret_and_selection(store, focus_byte, anchor_byte);
                }
                self.mark_a11y_dirty(target);
                self.rasterize_current();
                Ok(true)
            }
            A11yAction::ScrollIntoView => {
                let mut scrolled = Vec::new();
                let changed = {
                    let (store, backend) = self.replica.store_and_backend_mut();
                    backend.scroll_into_view_collecting(store, target, Some(&mut scrolled))
                };
                if !changed {
                    return Ok(false);
                }
                let ts = TimestampUs(now_us());
                let mut batch = EventBatchBuf::with_capacity(scrolled.len(), 0);
                for &scroll_id in &scrolled {
                    self.mark_a11y_dirty(scroll_id);
                    let (new_x, new_y) = self.replica.backend().scroll_offset(scroll_id);
                    let (cw, ch) = self
                        .replica
                        .backend()
                        .scroll_content_size(self.replica.store(), scroll_id)
                        .unwrap_or((self.viewport.width, self.viewport.height));
                    batch.push_scroll(self.surface_id, scroll_id, ts, new_x, new_y, cw, ch);
                }
                let handled = self.app.dispatch_events(batch.as_validated());
                if handled {
                    self.render_and_rasterize()?;
                } else {
                    self.rasterize_current();
                }
                Ok(true)
            }
            A11yAction::ScrollBy { dx, dy } => self.apply_a11y_scroll(target, dx, dy, false),
            A11yAction::SetScrollOffset { x, y } => self.apply_a11y_scroll(target, x, y, true),
            A11yAction::Increment => {
                self.dispatch_a11y_target_key(target, is_disabled, "ArrowUp")
            }
            A11yAction::Decrement => {
                self.dispatch_a11y_target_key(target, is_disabled, "ArrowDown")
            }
            A11yAction::ShowContextMenu => {
                self.dispatch_a11y_target_key(target, is_disabled, "ContextMenu")
            }
        }
    }

    fn dispatch_a11y_target_key(
        &mut self,
        target: NodeId,
        is_disabled: bool,
        key: &str,
    ) -> Result<bool, DesktopHostError> {
        if is_disabled {
            return Ok(false);
        }
        let mut changed = false;
        if self.replica.backend().focused_node() != Some(target)
            && is_focusable(self.replica.store(), target)
        {
            changed |= self.focus_node(Some(target))?;
        }
        changed |= self.handle_keydown_for_target(target, key, false, false, false, false)?;
        Ok(changed)
    }

    fn apply_a11y_scroll(
        &mut self,
        target: NodeId,
        a: f32,
        b: f32,
        absolute: bool,
    ) -> Result<bool, DesktopHostError> {
        let prev = self.replica.backend().scroll_offset(target);
        let (store, backend) = self.replica.store_and_backend_mut();
        let (new_x, new_y) = if absolute {
            backend.set_scroll_offset(store, target, a, b)
        } else {
            backend.apply_scroll_delta(store, target, a, b)
        };
        let offset_changed = (prev.0 - new_x).abs() > 1e-4 || (prev.1 - new_y).abs() > 1e-4;
        let (cw, ch) = backend
            .scroll_content_size(store, target)
            .unwrap_or((self.viewport.width, self.viewport.height));
        if offset_changed {
            self.mark_a11y_dirty(target);
        }
        let mut batch = EventBatchBuf::with_capacity(1, 0);
        batch.push_scroll(
            self.surface_id,
            target,
            TimestampUs(now_us()),
            new_x,
            new_y,
            cw,
            ch,
        );
        let handled = self.app.dispatch_events(batch.as_validated());
        if handled {
            self.render_and_rasterize()?;
        } else if offset_changed {
            self.rasterize_current();
        }
        Ok(handled || offset_changed)
    }
}

impl<A: DesktopApp> SurfaceWindow<A> {
    /// Pushes an incremental (or post-reload full) AccessKit update to the OS adapter if an
    /// assistive technology is currently active. Zero work & zero allocations when inactive.
    pub(crate) fn sync_a11y_state(&mut self, force_geometry_or_scroll: bool) {
        let cur_gen = self.session.store_generation();
        if self.a11y_synced_store_gen != cur_gen {
            self.a11y_synced_store_gen = cur_gen;
            self.a11y_bridge.advance_epoch();
        }
        if !self.a11y_bridge.is_active() {
            return;
        }
        if let Some(update) = self.session.sync_a11y_incremental(
            &mut self.a11y_bridge,
            &self.base_title,
            force_geometry_or_scroll,
        ) {
            self.a11y_adapter.update_if_active(|| update);
        }
    }
}

impl<A: DesktopApp> DesktopWindowHost<A> {
    /// Handles an asynchronous [`accesskit_winit::Event`] delivered through `EventLoopProxy`.
    pub(crate) fn handle_accesskit_event(&mut self, event: accesskit_winit::Event) {
        let window_id: WindowId = event.window_id;
        match event.window_event {
            accesskit_winit::WindowEvent::InitialTreeRequested => {
                if let Some(sw) = self.windows.get_mut(&window_id) {
                    sw.a11y_synced_store_gen = sw.session.store_generation();
                    sw.a11y_bridge.activate();
                    let update = sw
                        .session
                        .build_a11y_full_tree(&mut sw.a11y_bridge, &sw.base_title);
                    sw.a11y_adapter.update_if_active(|| update);
                }
            }
            accesskit_winit::WindowEvent::AccessibilityDeactivated => {
                if let Some(sw) = self.windows.get_mut(&window_id) {
                    sw.a11y_bridge.deactivate();
                }
            }
            accesskit_winit::WindowEvent::ActionRequested(req) => {
                let mut reschedule = false;
                if let Some(sw) = self.windows.get_mut(&window_id) {
                    if self.window_manager.active_modal_blocker(sw.handle_id).is_some() {
                        return;
                    }
                    let cur_gen = sw.session.store_generation();
                    if sw.a11y_synced_store_gen != cur_gen {
                        sw.a11y_synced_store_gen = cur_gen;
                        sw.a11y_bridge.advance_epoch();
                        sw.sync_a11y_state(true);
                        return;
                    }
                    if let Some(resolved) = sw.a11y_bridge.translate_action(&req) {
                        match sw.session.dispatch_a11y_action(resolved) {
                            Ok(true) => {
                                let (_, exhausted) = Self::pump_window_async(sw, false);
                                reschedule = exhausted;
                                sw.sync_ime_state();
                                sw.sync_cursor_state();
                                sw.sync_a11y_state(false);
                                sw.window.request_redraw();
                            }
                            Ok(false) => {}
                            Err(err) => {
                                eprintln!("[nve-skia-shell] AccessKit action error: {err}");
                            }
                        }
                    }
                }
                if reschedule {
                    self.host_services.request_frame_wake();
                }
            }
        }
    }
}
