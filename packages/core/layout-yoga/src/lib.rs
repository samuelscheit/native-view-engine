//! Real React Native Yoga C++20 [`StoreLayoutEngine`] implementation for `native-view-engine`.
//!
//! When built with `RN_REACT_COMMON_DIR=/path/to/react-native/packages/react-native/ReactCommon`,
//! `build.rs` compiles React Native's `yogacore` C++20 sources and links [`YogaLayoutEngine`]
//! directly to Yoga with full `YGNodeSetMeasureFunc` -> Rust [`TextMeasurer`] callbacks,
//! reading directly from `NodeStore` word tables (`Layout`, `TextContent`, `Typography`).
//!
//! When built without `RN_REACT_COMMON_DIR`, the crate still compiles cleanly on every machine
//! with [`YogaLayoutEngine::is_available()`] returning `false` and [`YogaLayoutEngine::new()`]
//! returning [`YogaUnavailableError`].

#![cfg_attr(doctest, doc = include_str!("../README.md"))]

#[cfg(nve_has_rn_yoga)]
mod ffi;

use std::fmt;

/// Error returned by [`YogaLayoutEngine::new`] when `nve-layout-yoga` was compiled without
/// `RN_REACT_COMMON_DIR` pointing to a React Native `ReactCommon/yoga` checkout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct YogaUnavailableError {
    message: &'static str,
}

impl YogaUnavailableError {
    pub const fn new() -> Self {
        Self {
            message: "React Native Yoga was not compiled into nve-layout-yoga; set RN_REACT_COMMON_DIR=/path/to/react-native/packages/react-native/ReactCommon and rebuild",
        }
    }

    pub const fn message(&self) -> &'static str {
        self.message
    }
}

impl Default for YogaUnavailableError {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for YogaUnavailableError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message)
    }
}

impl std::error::Error for YogaUnavailableError {}

#[cfg(nve_has_rn_yoga)]
mod real_impl {
    use std::collections::HashMap;
    use std::ffi::c_void;
    use std::panic::{catch_unwind, AssertUnwindSafe};
    use std::sync::Arc;

    use nve_tables::tables::{Layout, TextContent, Typography};
    use nve_engine::{HeuristicTextMeasurer, LayoutRect, StoreLayoutEngine, TextMeasurer, Viewport};
    use nve_store::{ChangeSet, NodeId, NodeStore};

    use crate::ffi::{
        nve_yg_config_free, nve_yg_config_new, nve_yg_config_set_point_scale_factor,
        nve_yg_node_apply_layout, nve_yg_node_apply_layout_ext, nve_yg_node_calculate_layout,
        nve_yg_node_detach_and_clear, nve_yg_node_free, nve_yg_node_get_layout_rect,
        nve_yg_node_mark_dirty, nve_yg_node_new_with_config, nve_yg_node_set_children,
        nve_yg_node_set_fixed_size, nve_yg_node_set_is_text, nve_yg_node_set_measure_hook,
        NveYogaLayoutExt, NveYogaLayoutProps, NveYogaMeasureHook, YGConfigRef, YGNodeRef,
    };
    use crate::YogaUnavailableError;

    const MEASURE_MODE_UNDEFINED: u8 = 0;
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
                MEASURE_MODE_AT_MOST if width.is_finite() => measured_w.min(width.max(0.0)),
                MEASURE_MODE_UNDEFINED | _ => measured_w,
            };
            let final_h = match height_mode {
                MEASURE_MODE_EXACTLY if height.is_finite() => height.max(0.0),
                MEASURE_MODE_AT_MOST if height.is_finite() => measured_h.min(height.max(0.0)),
                MEASURE_MODE_UNDEFINED | _ => measured_h,
            };
            (final_w, final_h)
        }));
        let (w, h) = result.unwrap_or((0.0, 0.0));
        unsafe {
            *out_width = w;
            *out_height = h;
        }
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
    }

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
            let config = unsafe { nve_yg_config_new() };
            assert!(!config.is_null(), "YGConfigNew returned null");
            Ok(Self {
                config,
                nodes: HashMap::new(),
                computed_rects: HashMap::new(),
                text_measurer,
                child_scratch: Vec::new(),
            })
        }

        fn ensure_text_measure_attached(entry: &mut YogaNodeEntry, measurer: &Arc<dyn TextMeasurer>) {
            let state = entry
                .measure_state
                .get_or_insert_with(|| BoxedMeasureState::new(Arc::clone(measurer)));
            let hook_ptr = state.hook_ptr();
            unsafe {
                nve_yg_node_set_is_text(entry.yg_node, true);
                nve_yg_node_set_measure_hook(entry.yg_node, hook_ptr);
            }
        }

        fn ensure_node(&mut self, store: &NodeStore, id: NodeId) {
            if !self.nodes.contains_key(&id) {
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
            let hidden_input = nve_tables::tables::is_hidden_text_input(store, id);
            let (props, ext_storage) = if let Some(layout) = store.try_row::<Layout>(id) {
                let props = NveYogaLayoutProps::from_store_layout(&layout);
                let mut ext = NveYogaLayoutExt::from_store_layout(&layout);
                if hidden_input {
                    ext.display = 1;
                }
                (props, Some(ext))
            } else {
                let props = NveYogaLayoutProps {
                    flex_shrink: 1.0,
                    ..Default::default()
                };
                let ext = hidden_input.then(|| NveYogaLayoutExt {
                    display: 1,
                    ..Default::default()
                });
                (props, ext)
            };
            let ext_ptr = ext_storage
                .as_ref()
                .map_or(std::ptr::null(), std::ptr::from_ref);
            // SAFETY: `entry.yg_node` is a valid Yoga node owned by `self.nodes`; `props` is a
            // valid stack reference and `ext_ptr` is either null or points to live `ext_storage`.
            unsafe {
                nve_yg_node_apply_layout(entry.yg_node, &props);
                nve_yg_node_apply_layout_ext(entry.yg_node, ext_ptr);
            }

            let text_row = store.try_row::<TextContent>(id);
            let is_text = text_row.is_some();
            if is_text {
                if !entry.is_text {
                    entry.is_text = true;
                    Self::ensure_text_measure_attached(entry, &self.text_measurer);
                }
                let text = text_row.map(|t| t.text()).unwrap_or("");
                let (font_size, font_weight) = store
                    .try_row::<Typography>(id)
                    .map(|t| {
                        let fs = t.font_size();
                        (
                            if fs.is_finite() && fs > 0.0 {
                                fs
                            } else {
                                HeuristicTextMeasurer::DEFAULT_FONT_SIZE
                            },
                            t.font_weight(),
                        )
                    })
                    .unwrap_or((HeuristicTextMeasurer::DEFAULT_FONT_SIZE, 400));
                if let Some(state) = entry.measure_state.as_mut() {
                    let changed = state.payload.text != text
                        || (state.payload.font_size - font_size).abs() > f32::EPSILON
                        || state.payload.font_weight != font_weight;
                    if changed {
                        state.payload.text.clear();
                        state.payload.text.push_str(text);
                        state.payload.font_size = font_size;
                        state.payload.font_weight = font_weight;
                        unsafe { nve_yg_node_mark_dirty(entry.yg_node) };
                    }
                }
            } else if entry.is_text {
                entry.is_text = false;
                unsafe {
                    nve_yg_node_set_is_text(entry.yg_node, false);
                    nve_yg_node_set_measure_hook(entry.yg_node, std::ptr::null_mut());
                }
                entry.measure_state = None;
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
            unsafe {
                nve_yg_node_set_children(
                    parent_yg,
                    self.child_scratch.as_ptr(),
                    self.child_scratch.len(),
                );
            }
        }

        fn sync_subtree_if_unseen(&mut self, store: &NodeStore, id: NodeId) {
            if !store.is_alive(id) {
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
                unsafe {
                    nve_yg_node_detach_and_clear(entry.yg_node);
                }
            }
            for (_, entry) in self.nodes.drain() {
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
            for &id in &changes.removed {
                if let Some(entry) = self.nodes.remove(&id) {
                    unsafe {
                        nve_yg_node_detach_and_clear(entry.yg_node);
                        nve_yg_node_free(entry.yg_node);
                    }
                    drop(entry.measure_state);
                }
                self.computed_rects.remove(&id);
            }
            for &id in &changes.created {
                if store.is_alive(id) {
                    self.ensure_node(store, id);
                }
            }
            for &id in &changes.touched {
                if store.is_alive(id) {
                    self.ensure_node(store, id);
                }
            }
            for &parent in &changes.children_changed {
                if store.is_alive(parent) {
                    self.ensure_node(store, parent);
                    self.sync_children_of(store, parent);
                }
            }
        }

        fn compute_layout(&mut self, store: &NodeStore, root: NodeId, viewport: Viewport) {
            if !store.is_alive(root) {
                self.computed_rects.clear();
                return;
            }
            if !self.nodes.contains_key(&root) {
                self.sync_subtree_if_unseen(store, root);
            }
            let Some(root_yg) = self.nodes.get(&root).map(|e| e.yg_node) else {
                self.computed_rects.clear();
                return;
            };
            unsafe {
                nve_yg_config_set_point_scale_factor(self.config, viewport.scale_factor);
                nve_yg_node_set_fixed_size(root_yg, viewport.width, viewport.height);
                nve_yg_node_calculate_layout(root_yg, viewport.width, viewport.height);
            }
            self.computed_rects.clear();
            self.collect_rects(store, root, 0.0, 0.0);
        }

        fn layout_of(&self, id: NodeId) -> Option<LayoutRect> {
            self.computed_rects.get(&id).copied()
        }

        fn set_text_measurer(&mut self, measurer: Arc<dyn TextMeasurer>) {
            self.text_measurer = Arc::clone(&measurer);
            for entry in self.nodes.values_mut() {
                if let Some(state) = entry.measure_state.as_mut() {
                    state.payload.measurer = Arc::clone(&measurer);
                    unsafe { nve_yg_node_mark_dirty(entry.yg_node) };
                }
            }
        }

        fn reset(&mut self) {
            self.clear_all_nodes();
        }
    }
}

#[cfg(not(nve_has_rn_yoga))]
mod unavailable_impl {
    use std::sync::Arc;

    use nve_engine::{LayoutRect, StoreLayoutEngine, TextMeasurer, Viewport};
    use nve_store::{ChangeSet, NodeId, NodeStore};

    use crate::YogaUnavailableError;

    /// Stub handle compiled when `RN_REACT_COMMON_DIR` was not set at build time.
    pub struct YogaLayoutEngine {
        _uninhabited: std::convert::Infallible,
    }

    impl YogaLayoutEngine {
        pub const fn is_available() -> bool {
            false
        }

        pub fn new() -> Result<Self, YogaUnavailableError> {
            Err(YogaUnavailableError::new())
        }

        pub fn with_text_measurer(
            _text_measurer: Arc<dyn TextMeasurer>,
        ) -> Result<Self, YogaUnavailableError> {
            Err(YogaUnavailableError::new())
        }
    }

    impl StoreLayoutEngine for YogaLayoutEngine {
        fn name(&self) -> &'static str {
            match self._uninhabited {}
        }

        fn sync_changes(&mut self, _store: &NodeStore, _changes: &ChangeSet) {
            match self._uninhabited {}
        }

        fn compute_layout(&mut self, _store: &NodeStore, _root: NodeId, _viewport: Viewport) {
            match self._uninhabited {}
        }

        fn layout_of(&self, _id: NodeId) -> Option<LayoutRect> {
            match self._uninhabited {}
        }

        fn set_text_measurer(&mut self, _measurer: Arc<dyn TextMeasurer>) {
            match self._uninhabited {}
        }

        fn reset(&mut self) {
            match self._uninhabited {}
        }
    }
}

#[cfg(nve_has_rn_yoga)]
pub use real_impl::YogaLayoutEngine;
#[cfg(not(nve_has_rn_yoga))]
pub use unavailable_impl::YogaLayoutEngine;

#[cfg(test)]
mod tests {
    use super::*;
    use nve_engine::StoreLayoutEngine;

    #[test]
    fn availability_matches_constructor() {
        if YogaLayoutEngine::is_available() {
            let engine = YogaLayoutEngine::new().expect("real RN Yoga should initialize");
            assert_eq!(engine.name(), "yoga");
        } else {
            let err = YogaLayoutEngine::new()
                .err()
                .expect("expected YogaUnavailableError");
            assert!(err.to_string().contains("RN_REACT_COMMON_DIR"));
        }
    }
}
