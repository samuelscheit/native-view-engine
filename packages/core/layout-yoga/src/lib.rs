//! Real React Native Yoga C++20 [`LayoutEngine`] implementation for `native-view-engine`.
//!
//! When built with `RN_REACT_COMMON_DIR=/path/to/react-native/packages/react-native/ReactCommon`,
//! `build.rs` compiles React Native's `yogacore` C++20 sources and links [`YogaLayoutEngine`]
//! directly to Yoga with full `YGNodeSetMeasureFunc` -> Rust [`TextMeasurer`] callbacks.
//!
//! When built without `RN_REACT_COMMON_DIR`, the crate still compiles cleanly on every machine
//! with [`YogaLayoutEngine::is_available()`] returning `false` and [`YogaLayoutEngine::new()`]
//! returning [`YogaUnavailableError`].

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

    use nve_abi::{ComponentKind, LayoutProps, NodeId, StyleProps};
    use nve_components::layout_ext::LayoutExt;
    use nve_engine::{HeuristicTextMeasurer, LayoutEngine, LayoutRect, TextMeasurer, Viewport};

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
        measurer: Arc<dyn TextMeasurer>,
    }

    /// Heap-stable container holding both the C++ `NveYogaMeasureHook` and the Rust payload
    /// referenced by `YGNodeSetContext`. Because it is always owned behind a `Box`, rehashing
    /// `HashMap<NodeId, YogaNodeEntry>` never moves or invalidates the pointer held by Yoga.
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
                    font_size: StyleProps::default().font_size,
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
            // SAFETY: `user_data` points to `BoxedMeasureState::payload`, which stays pinned on
            // the heap for the lifetime of the `YGNode` and is detached before drop.
            let payload = unsafe { &*(user_data.cast::<MeasurePayload>()) };
            if payload.text.is_empty() {
                return (0.0f32, 0.0f32);
            }

            let max_width = match width_mode {
                MEASURE_MODE_UNDEFINED => None,
                MEASURE_MODE_EXACTLY | MEASURE_MODE_AT_MOST => {
                    if width.is_finite() {
                        Some(width.max(0.0))
                    } else {
                        None
                    }
                }
                _ => None,
            };

            let (raw_w, raw_h) = payload
                .measurer
                .measure(&payload.text, payload.font_size, max_width);
            let mut measured_w = if raw_w.is_finite() && raw_w >= 0.0 {
                raw_w
            } else {
                0.0
            };
            let mut measured_h = if raw_h.is_finite() && raw_h >= 0.0 {
                raw_h
            } else {
                0.0
            };

            match width_mode {
                MEASURE_MODE_EXACTLY if width.is_finite() => measured_w = width.max(0.0),
                MEASURE_MODE_AT_MOST if width.is_finite() => {
                    measured_w = measured_w.min(width.max(0.0));
                }
                _ => {}
            }
            match height_mode {
                MEASURE_MODE_EXACTLY if height.is_finite() => measured_h = height.max(0.0),
                MEASURE_MODE_AT_MOST if height.is_finite() => {
                    measured_h = measured_h.min(height.max(0.0));
                }
                _ => {}
            }

            (measured_w, measured_h)
        }));

        let (w, h) = result.unwrap_or((0.0, 0.0));
        unsafe {
            *out_width = w;
            *out_height = h;
        }
    }

    struct YogaNodeEntry {
        yg_node: YGNodeRef,
        kind: ComponentKind,
        measure_state: Option<Box<BoxedMeasureState>>,
    }

    /// Real React Native Yoga C++20 implementation of [`LayoutEngine`].
    pub struct YogaLayoutEngine {
        config: YGConfigRef,
        nodes: HashMap<NodeId, YogaNodeEntry>,
        children: HashMap<NodeId, Vec<NodeId>>,
        computed_rects: HashMap<NodeId, LayoutRect>,
        text_measurer: Arc<dyn TextMeasurer>,
    }

    // SAFETY: `YogaLayoutEngine` exclusively owns its `YGConfigRef` and all `YGNodeRef`s;
    // no pointers are aliased across instances and all mutations require `&mut self`.
    unsafe impl Send for YogaLayoutEngine {}

    impl YogaLayoutEngine {
        /// Returns `true` because real React Native Yoga C++20 was compiled and linked.
        pub const fn is_available() -> bool {
            true
        }

        /// Creates a new [`YogaLayoutEngine`] backed by real React Native Yoga.
        pub fn new() -> Result<Self, YogaUnavailableError> {
            Self::with_text_measurer(Arc::new(HeuristicTextMeasurer))
        }

        /// Creates a new [`YogaLayoutEngine`] with a custom [`TextMeasurer`].
        pub fn with_text_measurer(
            text_measurer: Arc<dyn TextMeasurer>,
        ) -> Result<Self, YogaUnavailableError> {
            let config = unsafe { nve_yg_config_new() };
            assert!(!config.is_null(), "YGConfigNew returned null");
            Ok(Self {
                config,
                nodes: HashMap::new(),
                children: HashMap::new(),
                computed_rects: HashMap::new(),
                text_measurer,
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

        fn clear_all_nodes(&mut self) {
            // Detach children and hooks before freeing any Yoga node or dropping BoxedMeasureState.
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
            self.children.clear();
            self.computed_rects.clear();
        }

        fn collect_rects(&mut self, id: NodeId, origin_x: f32, origin_y: f32) {
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
            if let Some(child_ids) = self.children.get(&id).cloned() {
                for child in child_ids {
                    self.collect_rects(child, rect.x, rect.y);
                }
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

    impl LayoutEngine for YogaLayoutEngine {
        fn name(&self) -> &'static str {
            "yoga"
        }

        fn upsert_node(&mut self, id: NodeId, kind: ComponentKind) {
            if let Some(entry) = self.nodes.get_mut(&id) {
                if entry.kind != kind {
                    entry.kind = kind;
                    if kind == ComponentKind::Text {
                        Self::ensure_text_measure_attached(entry, &self.text_measurer);
                        unsafe { nve_yg_node_mark_dirty(entry.yg_node) };
                    } else {
                        unsafe {
                            nve_yg_node_set_is_text(entry.yg_node, false);
                            nve_yg_node_set_measure_hook(entry.yg_node, std::ptr::null_mut());
                        }
                        entry.measure_state = None;
                    }
                }
                return;
            }

            let yg_node = unsafe { nve_yg_node_new_with_config(self.config) };
            assert!(!yg_node.is_null(), "YGNodeNewWithConfig returned null");
            let default_props = NveYogaLayoutProps::from(&LayoutProps::default());
            unsafe {
                nve_yg_node_apply_layout(yg_node, &default_props);
            }

            let mut entry = YogaNodeEntry {
                yg_node,
                kind,
                measure_state: None,
            };
            if kind == ComponentKind::Text {
                Self::ensure_text_measure_attached(&mut entry, &self.text_measurer);
            }
            self.nodes.insert(id, entry);
        }

        fn remove_node(&mut self, id: NodeId) {
            if let Some(entry) = self.nodes.remove(&id) {
                unsafe {
                    nve_yg_node_free(entry.yg_node);
                }
                drop(entry.measure_state);
            }
            self.children.remove(&id);
            self.computed_rects.remove(&id);
        }

        fn set_layout(&mut self, id: NodeId, layout: &LayoutProps) {
            let Some(entry) = self.nodes.get(&id) else {
                return;
            };
            let props = NveYogaLayoutProps::from(layout);
            unsafe {
                nve_yg_node_apply_layout(entry.yg_node, &props);
            }
        }

        fn set_layout_ext(&mut self, id: NodeId, ext: Option<&LayoutExt>) {
            let Some(entry) = self.nodes.get(&id) else {
                return;
            };
            match ext {
                Some(e) => {
                    let ffi_ext = NveYogaLayoutExt::from(e);
                    unsafe {
                        nve_yg_node_apply_layout_ext(entry.yg_node, &ffi_ext);
                    }
                }
                None => unsafe {
                    nve_yg_node_apply_layout_ext(entry.yg_node, std::ptr::null());
                },
            }
        }

        fn set_style(&mut self, id: NodeId, style: &StyleProps) {
            let Some(entry) = self.nodes.get_mut(&id) else {
                return;
            };
            let font_size = if style.font_size.is_finite() && style.font_size > 0.0 {
                style.font_size
            } else {
                StyleProps::default().font_size
            };
            if entry.kind == ComponentKind::Text {
                Self::ensure_text_measure_attached(entry, &self.text_measurer);
                if let Some(state) = entry.measure_state.as_mut() {
                    if (state.payload.font_size - font_size).abs() > f32::EPSILON {
                        state.payload.font_size = font_size;
                        unsafe { nve_yg_node_mark_dirty(entry.yg_node) };
                    }
                }
            }
        }

        fn set_text(&mut self, id: NodeId, text: &str) {
            let Some(entry) = self.nodes.get_mut(&id) else {
                return;
            };
            if entry.kind == ComponentKind::Text {
                Self::ensure_text_measure_attached(entry, &self.text_measurer);
                if let Some(state) = entry.measure_state.as_mut() {
                    if state.payload.text != text {
                        state.payload.text.clear();
                        state.payload.text.push_str(text);
                        unsafe { nve_yg_node_mark_dirty(entry.yg_node) };
                    }
                }
            }
        }

        fn set_children(&mut self, parent: NodeId, children: &[NodeId]) {
            self.children.insert(parent, children.to_vec());
            let Some(parent_yg) = self.nodes.get(&parent).map(|e| e.yg_node) else {
                return;
            };
            let child_refs: Vec<YGNodeRef> = children
                .iter()
                .filter_map(|cid| self.nodes.get(cid).map(|e| e.yg_node))
                .collect();
            unsafe {
                nve_yg_node_set_children(parent_yg, child_refs.as_ptr(), child_refs.len());
            }
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

        fn compute_layout(&mut self, root: NodeId, viewport: Viewport) {
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
            self.collect_rects(root, 0.0, 0.0);
        }

        fn layout_of(&self, id: NodeId) -> Option<LayoutRect> {
            self.computed_rects.get(&id).copied()
        }

        fn reset(&mut self) {
            self.clear_all_nodes();
        }
    }
}

#[cfg(not(nve_has_rn_yoga))]
mod unavailable_impl {
    use std::sync::Arc;

    use nve_abi::{ComponentKind, LayoutProps, NodeId, StyleProps};
    use nve_engine::{LayoutEngine, LayoutRect, TextMeasurer, Viewport};

    use crate::YogaUnavailableError;

    /// Stub handle compiled when `RN_REACT_COMMON_DIR` was not set at build time.
    ///
    /// [`YogaLayoutEngine::is_available`] returns `false` and [`YogaLayoutEngine::new`] returns
    /// [`YogaUnavailableError`], allowing workspace crates and generic code to compile cleanly
    /// on machines without a local React Native checkout.
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

    impl LayoutEngine for YogaLayoutEngine {
        fn name(&self) -> &'static str {
            match self._uninhabited {}
        }

        fn upsert_node(&mut self, _id: NodeId, _kind: ComponentKind) {
            match self._uninhabited {}
        }

        fn remove_node(&mut self, _id: NodeId) {
            match self._uninhabited {}
        }

        fn set_layout(&mut self, _id: NodeId, _layout: &LayoutProps) {
            match self._uninhabited {}
        }

        fn set_style(&mut self, _id: NodeId, _style: &StyleProps) {
            match self._uninhabited {}
        }

        fn set_text(&mut self, _id: NodeId, _text: &str) {
            match self._uninhabited {}
        }

        fn set_children(&mut self, _parent: NodeId, _children: &[NodeId]) {
            match self._uninhabited {}
        }

        fn set_text_measurer(&mut self, _measurer: Arc<dyn TextMeasurer>) {
            match self._uninhabited {}
        }

        fn compute_layout(&mut self, _root: NodeId, _viewport: Viewport) {
            match self._uninhabited {}
        }

        fn layout_of(&self, _id: NodeId) -> Option<LayoutRect> {
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
    use nve_engine::LayoutEngine;

    #[test]
    fn availability_matches_constructor() {
        if YogaLayoutEngine::is_available() {
            let engine = YogaLayoutEngine::new().expect("real RN Yoga should initialize");
            assert_eq!(engine.name(), "yoga");
        } else {
            let err = YogaLayoutEngine::new().err().expect("expected YogaUnavailableError");
            assert!(err.to_string().contains("RN_REACT_COMMON_DIR"));
        }
    }
}
