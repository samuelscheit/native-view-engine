//! In-frame extended typed-property handlers for [`WebBackend`]'s [`nve_engine::RenderSink`]
//! implementation (coalesced into a single end-of-frame style/DOM/canvas/SVG flush).

use nve_abi::NodeId;
use nve_components::canvas_ir::DrawList;
use nve_components::interaction::{CursorKind, StateStyleRule};
use nve_components::layout_ext::LayoutExt;
use nve_components::media::{MediaQuery, ResponsiveRule};
use nve_components::motion::TransitionSpec;
use nve_components::native_view::TypedNativeViewProps;
use nve_components::paint::BackgroundFill;
use nve_components::svg::SvgPicture;
use nve_components::transform::Transform2D;

#[cfg(target_arch = "wasm32")]
use crate::layout_ext_css::node_dir_attr;
use crate::WebBackend;

impl WebBackend {
    pub(crate) fn apply_frame_layout_ext(&mut self, node_id: NodeId, ext: Option<&LayoutExt>) {
        let next = ext.filter(|e| !e.is_default()).map(|e| Box::new(e.clone()));
        let Some(node) = self.nodes.get_mut(&node_id) else {
            return;
        };
        if node.layout_ext != next {
            #[cfg(target_arch = "wasm32")]
            if node_dir_attr(node.layout_ext.as_deref()) != node_dir_attr(next.as_deref()) {
                self.frame_dirty_dirs.push(node_id);
            }
            node.layout_ext = next;
            self.frame_dirty_styles.push(node_id);
        }
    }

    pub(crate) fn apply_frame_transform(&mut self, node_id: NodeId, transform: Option<Transform2D>) {
        let Some(node) = self.nodes.get_mut(&node_id) else {
            return;
        };
        if node.transform != transform {
            node.transform = transform;
            self.frame_dirty_styles.push(node_id);
        }
    }

    pub(crate) fn apply_frame_cursor(&mut self, node_id: NodeId, cursor: Option<CursorKind>) {
        let Some(node) = self.nodes.get_mut(&node_id) else {
            return;
        };
        if node.cursor != cursor {
            node.cursor = cursor;
            self.frame_dirty_styles.push(node_id);
        }
    }

    pub(crate) fn apply_frame_state_rules(&mut self, node_id: NodeId, rules: &[StateStyleRule]) {
        let Some(node) = self.nodes.get_mut(&node_id) else {
            return;
        };
        if node.state_rules.as_slice() != rules {
            node.state_rules.clear();
            node.state_rules.extend_from_slice(rules);
            self.frame_dirty_styles.push(node_id);
        }
    }

    pub(crate) fn apply_frame_transitions(&mut self, node_id: NodeId, transitions: &[TransitionSpec]) {
        let Some(node) = self.nodes.get_mut(&node_id) else {
            return;
        };
        if node.transitions.as_slice() != transitions {
            node.transitions.clear();
            node.transitions.extend_from_slice(transitions);
            self.frame_dirty_styles.push(node_id);
        }
    }

    pub(crate) fn apply_frame_show_on(&mut self, node_id: NodeId, query: Option<MediaQuery>) {
        let Some(node) = self.nodes.get_mut(&node_id) else {
            return;
        };
        if node.show_on != query {
            node.show_on = query;
            self.frame_dirty_styles.push(node_id);
        }
    }

    pub(crate) fn apply_frame_responsive_rules(
        &mut self,
        node_id: NodeId,
        rules: &[ResponsiveRule],
    ) {
        let Some(node) = self.nodes.get_mut(&node_id) else {
            return;
        };
        if node.responsive_rules.as_slice() != rules {
            node.responsive_rules.clear();
            node.responsive_rules.extend_from_slice(rules);
            self.frame_dirty_styles.push(node_id);
        }
    }

    pub(crate) fn apply_frame_background_fill(
        &mut self,
        node_id: NodeId,
        fill: Option<&BackgroundFill>,
    ) {
        let Some(node) = self.nodes.get_mut(&node_id) else {
            return;
        };
        if node.background_fill.as_ref() != fill {
            node.background_fill = fill.cloned();
            self.frame_dirty_styles.push(node_id);
        }
    }

    pub(crate) fn apply_frame_svg(&mut self, node_id: NodeId, svg: Option<&SvgPicture>) {
        let Some(node) = self.nodes.get_mut(&node_id) else {
            return;
        };
        let unchanged = match (&node.svg, svg) {
            (None, None) => true,
            (Some(a), Some(b)) => a.ptr_eq(b) || a == b,
            _ => false,
        };
        if !unchanged {
            node.svg = svg.cloned();
            #[cfg(target_arch = "wasm32")]
            self.frame_dirty_svgs.push(node_id);
        }
    }

    pub(crate) fn apply_frame_draw_list(&mut self, node_id: NodeId, draw_list: Option<&DrawList>) {
        let Some(el) = self.nodes.get_mut(&node_id) else {
            return;
        };
        match draw_list {
            Some(next) => {
                if !(el.draw_ops.ptr_eq(next) || el.draw_ops == *next) {
                    el.draw_op_count = next.len();
                    el.draw_ops = next.clone();
                    #[cfg(target_arch = "wasm32")]
                    self.frame_dirty_canvases.push(node_id);
                }
            }
            None => {
                if !el.draw_ops.is_empty() {
                    el.draw_op_count = 0;
                    el.draw_ops = DrawList::empty();
                    #[cfg(target_arch = "wasm32")]
                    self.frame_dirty_canvases.push(node_id);
                }
            }
        }
    }

    pub(crate) fn apply_frame_native_view_props(
        &mut self,
        node_id: NodeId,
        props: Option<&TypedNativeViewProps>,
    ) {
        #[cfg(any(
            feature = "nv-blur",
            feature = "nv-webview",
            feature = "nv-video",
            feature = "nv-web-element",
            feature = "camera",
        ))]
        {
            if let Some(p) = props {
                let mut dirty = std::mem::take(&mut self.frame_dirty_styles);
                self.apply_typed_native_view_props_in_frame(node_id, p, &mut dirty);
                self.frame_dirty_styles = dirty;
            }
        }
        #[cfg(not(any(
            feature = "nv-blur",
            feature = "nv-webview",
            feature = "nv-video",
            feature = "nv-web-element",
            feature = "camera",
        )))]
        let _ = (node_id, props);
    }
}
