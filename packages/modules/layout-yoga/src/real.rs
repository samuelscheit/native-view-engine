//! Real React Native Yoga C++20 [`StoreLayoutEngine`] implementation.

use std::collections::HashMap;
use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use nve_components::tables::{
    FlexDirection, Layout, Overflow, Scroll, TextContent, Typography, Visual,
};
use nve_engine::{LayoutRect, Viewport};
use nve_layout::{HeuristicTextMeasurer, StoreLayoutEngine, TextMeasurer};
use nve_store::{ChangeSet, NodeId, NodeStore, NveTable};

use crate::YogaUnavailableError;
use crate::ffi::{
    NveYogaLayoutExt, NveYogaLayoutProps, NveYogaMeasureHook, YGConfigRef, YGNodeRef,
    nve_yg_config_free, nve_yg_config_new, nve_yg_config_set_point_scale_factor,
    nve_yg_node_apply_layout, nve_yg_node_apply_layout_ext, nve_yg_node_calculate_layout,
    nve_yg_node_detach_and_clear, nve_yg_node_free, nve_yg_node_get_layout_rect,
    nve_yg_node_mark_dirty, nve_yg_node_new_with_config, nve_yg_node_set_children,
    nve_yg_node_set_fixed_size, nve_yg_node_set_is_text, nve_yg_node_set_measure_hook,
};

const MEASURE_MODE_EXACTLY: u8 = 1;
const MEASURE_MODE_AT_MOST: u8 = 2;

struct MeasurePayload {
    text: String,
    font_size: f32,
    font_weight: u16,
    measurer: Arc<dyn TextMeasurer>,
}

struct BoxedMeasureState {
    hook: NveYogaMeasureHook,
    payload: MeasurePayload,
}

impl BoxedMeasureState {
    fn new(measurer: Arc<dyn TextMeasurer>) -> Box<Self> {
        let mut boxed = Box::new(Self {
            hook: NveYogaMeasureHook {
                callback: rust_measure_callback,
                user_data: std::ptr::null_mut(),
            },
            payload: MeasurePayload {
                text: String::new(),
                font_size: HeuristicTextMeasurer::DEFAULT_FONT_SIZE,
                font_weight: 400,
                measurer,
            },
        });
        let payload_ptr = (&mut boxed.payload as *mut MeasurePayload).cast::<c_void>();
        boxed.hook.user_data = payload_ptr;
        boxed
    }

    fn hook_ptr(&mut self) -> *mut NveYogaMeasureHook {
        &mut self.hook as *mut NveYogaMeasureHook
    }
}

extern "C" fn rust_measure_callback(
    user_data: *mut c_void,
    width: f32,
    width_mode: u8,
    height: f32,
    height_mode: u8,
    out_width: *mut f32,
    out_height: *mut f32,
) {
    if user_data.is_null() || out_width.is_null() || out_height.is_null() {
        return;
    }
    let result = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: `user_data` was initialized from `&mut BoxedMeasureState::payload` and
        // remains live and unmoved for the entire lifetime of the attached `YGNode`.
        let payload = unsafe { &*(user_data.cast::<MeasurePayload>()) };
        let max_width = match width_mode {
            MEASURE_MODE_EXACTLY | MEASURE_MODE_AT_MOST if width.is_finite() => {
                Some(width.max(0.0))
            }
            _ => None,
        };
        let (measured_w, measured_h) = payload.measurer.measure_with_weight(
            &payload.text,
            payload.font_size,
            payload.font_weight,
            max_width,
        );
        let final_w = match width_mode {
            MEASURE_MODE_EXACTLY if width.is_finite() => width.max(0.0),
            _ => measured_w,
        };
        let final_h = match height_mode {
            MEASURE_MODE_EXACTLY if height.is_finite() => height.max(0.0),
            _ => measured_h,
        };
        (final_w, final_h)
    }));
    let (w, h) = result.unwrap_or((0.0, 0.0));
    // SAFETY: `out_width` and `out_height` are non-null stack pointers passed by
    // `nve_yoga_measure_trampoline` in `yoga_shim.cpp`.
    unsafe {
        *out_width = w;
        *out_height = h;
    }
}

/// Returns `true` when `id` is an auto-sized, visible-overflow child along a scroll container's
/// scrolling main axis. Per CSS Flexbox §4.5, such an item's automatic minimum size equals its
/// wrapped content size so it must not shrink inside the scroll viewport; Yoga's vertical
/// min-content walk omits cross-axis wrapping and gaps, so we preserve CSS semantics via
/// `flex_shrink = 0.0` (matching React Native `<ScrollView>` content containers).
fn preserve_scroll_child_intrinsic_size(store: &NodeStore, id: NodeId) -> bool {
    let Some(parent) = store.parent(id) else {
        return false;
    };
    let p_dir = store
        .try_row::<Layout>(parent)
        .map_or(FlexDirection::Column, |l| l.flex_direction());
    let is_col = matches!(p_dir, FlexDirection::Column | FlexDirection::ColumnReverse);
    let has_scroll_table = store.try_row::<Scroll>(parent).is_some();
    let scroll_main = has_scroll_table
        || store.try_row::<Visual>(parent).is_some_and(|p_vis| {
            if is_col {
                matches!(p_vis.overflow_y(), Overflow::Scroll | Overflow::Auto)
            } else {
                matches!(p_vis.overflow_x(), Overflow::Scroll | Overflow::Auto)
            }
        });
    if !scroll_main {
        return false;
    }
    let child_overflow_visible = store.try_row::<Visual>(id).is_none_or(|v| {
        if is_col {
            v.overflow_y() == Overflow::Visible
        } else {
            v.overflow_x() == Overflow::Visible
        }
    });
    if !child_overflow_visible {
        return false;
    }
    let Some(l) = store.try_row::<Layout>(id) else {
        return true;
    };
    let ar = l.aspect_ratio();
    if ar.is_finite() && ar > 0.0 {
        return false;
    }
    let (main_size, min_main) = if is_col {
        (l.height(), l.min_height())
    } else {
        (l.width(), l.min_width())
    };
    main_size.is_auto() && l.flex_basis().is_auto() && min_main.is_auto()
}

struct YogaNodeEntry {
    yg_node: YGNodeRef,
    is_text: bool,
    measure_state: Option<Box<BoxedMeasureState>>,
}

/// Real React Native Yoga C++20 implementation of [`StoreLayoutEngine`].
pub struct YogaLayoutEngine {
    config: YGConfigRef,
    nodes: HashMap<NodeId, YogaNodeEntry>,
    computed_rects: HashMap<NodeId, LayoutRect>,
    text_measurer: Arc<dyn TextMeasurer>,
    child_scratch: Vec<YGNodeRef>,
    /// Reusable stack for inherited typography propagation `(node_id, font_size, font_weight)`.
    typo_stack: Vec<(NodeId, f32, u16)>,
    /// Set when structure or `Typography` changed so text nodes refresh inherited font metrics.
    typo_dirty: bool,
    /// `StoreLayoutEngine::compute_layout` self-gate: set by every layout-affecting change,
    /// measurer swap, and reset; cleared only after a completed pass.
    layout_dirty: bool,
    /// `(root, width, height, scale)` of the last completed pass.
    last_pass: Option<(NodeId, f32, f32, f32)>,
}

// SAFETY: `YogaLayoutEngine` exclusively owns its `YGConfigRef`, all `YGNodeRef`s, and every
// boxed `MeasurePayload` (`TextMeasurer: Send + Sync`). Yoga nodes have no thread affinity and
// are never shared across threads concurrently (`&mut self` required for all mutations).
unsafe impl Send for YogaLayoutEngine {}

impl YogaLayoutEngine {
    pub const fn is_available() -> bool {
        true
    }

    pub fn new() -> Result<Self, YogaUnavailableError> {
        Self::with_text_measurer(Arc::new(HeuristicTextMeasurer))
    }

    pub fn with_text_measurer(
        text_measurer: Arc<dyn TextMeasurer>,
    ) -> Result<Self, YogaUnavailableError> {
        // SAFETY: Allocates a fresh `YGConfigRef` freed in `Drop`.
        let config = unsafe { nve_yg_config_new() };
        assert!(!config.is_null(), "YGConfigNew returned null");
        Ok(Self {
            config,
            nodes: HashMap::new(),
            computed_rects: HashMap::new(),
            text_measurer,
            child_scratch: Vec::new(),
            typo_stack: Vec::new(),
            typo_dirty: true,
            layout_dirty: true,
            last_pass: None,
        })
    }

    fn ensure_text_measure_attached(entry: &mut YogaNodeEntry, measurer: &Arc<dyn TextMeasurer>) {
        let state = entry
            .measure_state
            .get_or_insert_with(|| BoxedMeasureState::new(Arc::clone(measurer)));
        let hook_ptr = state.hook_ptr();
        // SAFETY: `entry.yg_node` is a live owned Yoga node and `hook_ptr` points into the
        // heap-allocated `Box<BoxedMeasureState>` owned by `entry` (detached before free).
        unsafe {
            nve_yg_node_set_measure_hook(entry.yg_node, hook_ptr);
            nve_yg_node_set_is_text(entry.yg_node, false);
        }
    }

    fn ensure_node(&mut self, store: &NodeStore, id: NodeId) {
        if !self.nodes.contains_key(&id) {
            // SAFETY: `self.config` is a live `YGConfigRef` owned by `self`.
            let yg_node = unsafe { nve_yg_node_new_with_config(self.config) };
            assert!(!yg_node.is_null(), "YGNodeNewWithConfig returned null");
            self.nodes.insert(
                id,
                YogaNodeEntry {
                    yg_node,
                    is_text: false,
                    measure_state: None,
                },
            );
        }
        self.sync_node_props(store, id);
    }

    fn sync_node_props(&mut self, store: &NodeStore, id: NodeId) {
        let Some(entry) = self.nodes.get_mut(&id) else {
            return;
        };
        let hidden_input = nve_components::tables::is_hidden_text_input(store, id);
        let layout = store.try_row::<Layout>(id);
        let visual = store.try_row::<Visual>(id);
        let mut props = if let Some(ref l) = layout {
            NveYogaLayoutProps::from_store_layout(l)
        } else {
            NveYogaLayoutProps {
                flex_shrink: 1.0,
                ..Default::default()
            }
        };
        if props.flex_shrink > 0.0 && preserve_scroll_child_intrinsic_size(store, id) {
            props.flex_shrink = 0.0;
        }
        let mut ext = NveYogaLayoutExt::from_store(layout.as_ref(), visual.as_ref());
        if hidden_input {
            ext.display = 1;
        }
        // SAFETY: `entry.yg_node` is a valid Yoga node owned by `self.nodes`; `props` and
        // `ext` are valid stack references.
        unsafe {
            nve_yg_node_apply_layout(entry.yg_node, &props);
            nve_yg_node_apply_layout_ext(entry.yg_node, &ext);
        }

        let text_row = store.try_row::<TextContent>(id);
        let is_text = text_row.is_some();
        if is_text {
            if !entry.is_text {
                entry.is_text = true;
                Self::ensure_text_measure_attached(entry, &self.text_measurer);
            }
            let text = text_row.map(|t| t.text()).unwrap_or("");
            if let Some(state) = entry.measure_state.as_mut()
                && state.payload.text != text
            {
                state.payload.text.clear();
                state.payload.text.push_str(text);
                // SAFETY: `entry.yg_node` is a live owned Yoga node.
                unsafe { nve_yg_node_mark_dirty(entry.yg_node) };
            }
        } else if entry.is_text {
            entry.is_text = false;
            // SAFETY: `entry.yg_node` is a live owned Yoga node; clearing hook before dropping
            // `measure_state` ensures Yoga never dereferences freed user_data.
            unsafe {
                nve_yg_node_set_is_text(entry.yg_node, false);
                nve_yg_node_set_measure_hook(entry.yg_node, std::ptr::null_mut());
            }
            entry.measure_state = None;
        }
    }

    /// Propagates inherited `Typography` (`font_size`, `font_weight`) from `root` down into
    /// every text node's Yoga measure payload, matching `TaffyStoreLayout::propagate_inheritance`.
    fn propagate_typography(&mut self, store: &NodeStore, root: NodeId) {
        if !self.typo_dirty {
            return;
        }
        self.typo_dirty = false;
        self.typo_stack.clear();
        self.typo_stack
            .push((root, HeuristicTextMeasurer::DEFAULT_FONT_SIZE, 400));

        while let Some((id, parent_fs, parent_fw)) = self.typo_stack.pop() {
            if !store.is_live(id) {
                continue;
            }
            let mut fs = parent_fs;
            let mut fw = parent_fw;
            if let Some(t) = store.try_row::<Typography>(id) {
                let set = t.set();
                if set & Typography::SET_FONT_SIZE != 0
                    || (set == 0 && t.font_size().to_bits() != 16.0f32.to_bits())
                {
                    let raw_fs = t.font_size();
                    if raw_fs.is_finite() && raw_fs > 0.0 {
                        fs = raw_fs;
                    }
                }
                if set & Typography::SET_FONT_WEIGHT != 0 || (set == 0 && t.font_weight() != 400) {
                    fw = t.font_weight();
                }
            }
            if let Some(entry) = self.nodes.get_mut(&id)
                && let Some(state) = entry.measure_state.as_mut()
            {
                let changed = (state.payload.font_size - fs).abs() > f32::EPSILON
                    || state.payload.font_weight != fw;
                if changed {
                    state.payload.font_size = fs;
                    state.payload.font_weight = fw;
                    self.layout_dirty = true;
                    // SAFETY: `entry.yg_node` is a live owned Yoga node.
                    unsafe { nve_yg_node_mark_dirty(entry.yg_node) };
                }
            }
            for &child in store.children(id).iter().rev() {
                self.typo_stack.push((child, fs, fw));
            }
        }
    }

    fn sync_children_of(&mut self, store: &NodeStore, parent: NodeId) {
        let Some(parent_yg) = self.nodes.get(&parent).map(|e| e.yg_node) else {
            return;
        };
        self.child_scratch.clear();
        for &cid in store.children(parent) {
            if let Some(child_entry) = self.nodes.get(&cid) {
                self.child_scratch.push(child_entry.yg_node);
            }
        }
        // SAFETY: `parent_yg` and every handle in `self.child_scratch` are live owned Yoga
        // nodes in `self.nodes`; the slice pointer is valid for `self.child_scratch.len()`.
        unsafe {
            nve_yg_node_set_children(
                parent_yg,
                self.child_scratch.as_ptr(),
                self.child_scratch.len(),
            );
        }
    }

    fn sync_subtree_if_unseen(&mut self, store: &NodeStore, id: NodeId) {
        if !store.is_live(id) {
            return;
        }
        self.ensure_node(store, id);
        let children: Vec<NodeId> = store.children(id).to_vec();
        for child in children {
            self.sync_subtree_if_unseen(store, child);
        }
        self.sync_children_of(store, id);
    }

    fn clear_all_nodes(&mut self) {
        for entry in self.nodes.values() {
            // SAFETY: Detaches every live Yoga node from its parent/children and clears its
            // measure callback before any node is freed in the next loop.
            unsafe {
                nve_yg_node_detach_and_clear(entry.yg_node);
            }
        }
        for (_, entry) in self.nodes.drain() {
            // SAFETY: Every `yg_node` was detached above and is freed exactly once here,
            // before its `measure_state` is dropped.
            unsafe {
                nve_yg_node_free(entry.yg_node);
            }
            drop(entry.measure_state);
        }
        self.computed_rects.clear();
    }

    fn collect_rects(&mut self, store: &NodeStore, id: NodeId, origin_x: f32, origin_y: f32) {
        let Some(entry) = self.nodes.get(&id) else {
            return;
        };
        // SAFETY: `entry.yg_node` is a live owned Yoga node after `calculate_layout`.
        let raw = unsafe { nve_yg_node_get_layout_rect(entry.yg_node) };
        let rect = LayoutRect {
            x: origin_x + raw.x,
            y: origin_y + raw.y,
            width: raw.width,
            height: raw.height,
        };
        self.computed_rects.insert(id, rect);
        for &child in store.children(id) {
            self.collect_rects(store, child, rect.x, rect.y);
        }
    }
}

impl Drop for YogaLayoutEngine {
    fn drop(&mut self) {
        self.clear_all_nodes();
        if !self.config.is_null() {
            // SAFETY: All nodes using `self.config` were freed by `clear_all_nodes()` above;
            // freeing `self.config` once here leaves no dangling references.
            unsafe {
                nve_yg_config_free(self.config);
            }
            self.config = std::ptr::null_mut();
        }
    }
}

impl StoreLayoutEngine for YogaLayoutEngine {
    fn name(&self) -> &'static str {
        "yoga"
    }

    fn sync_changes(&mut self, store: &NodeStore, changes: &ChangeSet) {
        let layout_effect = nve_store::effect::LAYOUT | nve_store::effect::MEASURE;
        let typo_changed = changes.table(Typography::slot()).is_some();
        if !changes.created.is_empty()
            || !changes.removed.is_empty()
            || !changes.children.is_empty()
            || typo_changed
        {
            self.typo_dirty = true;
        }
        if !changes.created.is_empty()
            || !changes.removed.is_empty()
            || !changes.children.is_empty()
            || typo_changed
            || changes.effects.iter().any(|&fx| fx & layout_effect != 0)
        {
            self.layout_dirty = true;
        }
        for &id in &changes.removed {
            if let Some(entry) = self.nodes.remove(&id) {
                // SAFETY: `entry.yg_node` is exclusively owned, detached before free, and
                // freed before `entry.measure_state` is dropped.
                unsafe {
                    nve_yg_node_detach_and_clear(entry.yg_node);
                    nve_yg_node_free(entry.yg_node);
                }
                drop(entry.measure_state);
            }
            self.computed_rects.remove(&id);
        }
        for &id in &changes.created {
            if store.is_live(id) {
                self.ensure_node(store, id);
            }
        }
        // Attach children already present on newly created nodes (e.g. live engine swap or
        // snapshot hydrate where `changes.created` lists the live tree without `changes.children`).
        for &id in &changes.created {
            if store.is_live(id) && !store.children(id).is_empty() {
                self.sync_children_of(store, id);
            }
        }
        for &id in &changes.touched {
            if store.is_live(id) {
                self.ensure_node(store, id);
                if !store.children(id).is_empty() {
                    let kids: Vec<NodeId> = store.children(id).to_vec();
                    for cid in kids {
                        if self.nodes.contains_key(&cid) {
                            self.sync_node_props(store, cid);
                        }
                    }
                }
            }
        }
        for &parent in &changes.children {
            if store.is_live(parent) {
                self.ensure_node(store, parent);
                if !store.children(parent).is_empty() {
                    let kids: Vec<NodeId> = store.children(parent).to_vec();
                    for cid in kids {
                        if self.nodes.contains_key(&cid) {
                            self.sync_node_props(store, cid);
                        }
                    }
                }
                self.sync_children_of(store, parent);
            }
        }
    }

    fn compute_layout(&mut self, store: &NodeStore, root: NodeId, viewport: Viewport) -> bool {
        if !store.is_live(root) {
            self.computed_rects.clear();
            self.layout_dirty = true;
            return true;
        }
        let pass_key = (root, viewport.width, viewport.height, viewport.scale_factor);
        if !self.nodes.contains_key(&root) {
            self.sync_subtree_if_unseen(store, root);
            self.typo_dirty = true;
        }
        self.propagate_typography(store, root);
        if !self.layout_dirty && self.last_pass == Some(pass_key) {
            return false;
        }
        let Some(root_yg) = self.nodes.get(&root).map(|e| e.yg_node) else {
            self.computed_rects.clear();
            self.layout_dirty = true;
            return true;
        };
        // SAFETY: `self.config` and `root_yg` are live owned Yoga handles; all attached
        // child nodes and `BoxedMeasureState` payloads remain live throughout layout.
        unsafe {
            nve_yg_config_set_point_scale_factor(self.config, viewport.scale_factor);
            nve_yg_node_set_fixed_size(root_yg, viewport.width, viewport.height);
            nve_yg_node_calculate_layout(root_yg, viewport.width, viewport.height);
        }
        self.computed_rects.clear();
        self.collect_rects(store, root, 0.0, 0.0);
        self.last_pass = Some(pass_key);
        self.layout_dirty = false;
        true
    }

    fn layout_of(&self, id: NodeId) -> Option<LayoutRect> {
        self.computed_rects.get(&id).copied()
    }

    fn set_text_measurer(&mut self, measurer: Arc<dyn TextMeasurer>) {
        self.text_measurer = Arc::clone(&measurer);
        for entry in self.nodes.values_mut() {
            if let Some(state) = entry.measure_state.as_mut() {
                state.payload.measurer = Arc::clone(&measurer);
                // SAFETY: `entry.yg_node` is a live owned Yoga node.
                unsafe { nve_yg_node_mark_dirty(entry.yg_node) };
            }
        }
        self.layout_dirty = true;
    }

    fn reset(&mut self) {
        self.clear_all_nodes();
        self.typo_dirty = true;
        self.layout_dirty = true;
        self.last_pass = None;
    }
}
