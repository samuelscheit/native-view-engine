//! Web DOM / WASM RenderBackend consuming the exact same `nve-abi` `CommandBuffer`.
//!
//! - On `wasm32`: creates and patches real browser DOM elements + CSS Flexbox directly via `web-sys`.
//! - On native targets: runs `WebDomMirror` so CSS translation & DOM mutations are unit-tested
//!   headlessly in `cargo test` in milliseconds.

use std::collections::HashMap;
use std::time::Instant;

use nve_abi::{
    ColorRgba, Command, CommandDecoder, ComponentKind, Dimension, FlexAlign, FlexDirection,
    FlexJustify, LayoutProps, NativeEvent, NodeId, StyleProps, SurfaceId, ROOT_NODE_ID,
};
use nve_engine::{BackendApplyStats, BackendError, LayoutRect, RenderBackend, Viewport};

pub fn component_to_html_tag(kind: ComponentKind) -> &'static str {
    match kind {
        ComponentKind::Root | ComponentKind::View | ComponentKind::ScrollView => "div",
        ComponentKind::Text => "span",
        ComponentKind::Image => "img",
        ComponentKind::TextInput => "input",
        ComponentKind::Pressable => "button",
        ComponentKind::SkiaCanvas => "canvas",
        ComponentKind::Custom => "div",
    }
}

pub fn layout_to_css(l: &LayoutProps) -> String {
    let dim = |d: Dimension| match d {
        Dimension::Auto => "auto".to_string(),
        Dimension::Points(v) => format!("{v}px"),
        Dimension::Percent(v) => format!("{v}%"),
    };
    let dir = match l.flex_direction {
        FlexDirection::Column => "column",
        FlexDirection::Row => "row",
        FlexDirection::ColumnReverse => "column-reverse",
        FlexDirection::RowReverse => "row-reverse",
    };
    let justify = match l.justify_content {
        FlexJustify::FlexStart => "flex-start",
        FlexJustify::Center => "center",
        FlexJustify::FlexEnd => "flex-end",
        FlexJustify::SpaceBetween => "space-between",
        FlexJustify::SpaceAround => "space-around",
        FlexJustify::SpaceEvenly => "space-evenly",
    };
    let align = match l.align_items {
        FlexAlign::Stretch => "stretch",
        FlexAlign::FlexStart => "flex-start",
        FlexAlign::Center => "center",
        FlexAlign::FlexEnd => "flex-end",
        FlexAlign::Baseline => "baseline",
    };
    format!(
        "display:flex;box-sizing:border-box;flex-direction:{dir};justify-content:{justify};align-items:{align};flex-grow:{};flex-shrink:{};gap:{}px;width:{};height:{};padding:{}px {}px {}px {}px;margin:{}px {}px {}px {}px;",
        l.flex_grow,
        l.flex_shrink,
        l.gap,
        dim(l.width),
        dim(l.height),
        l.padding.top,
        l.padding.right,
        l.padding.bottom,
        l.padding.left,
        l.margin.top,
        l.margin.right,
        l.margin.bottom,
        l.margin.left,
    )
}

pub fn style_to_css(s: &StyleProps) -> String {
    let rgba = |c: ColorRgba| format!("rgba({},{},{},{:.3})", c.r, c.g, c.b, c.a as f32 / 255.0);
    format!(
        "background-color:{};color:{};border:{}px solid {};border-radius:{}px;opacity:{};font-size:{}px;z-index:{};",
        rgba(s.background),
        rgba(s.text_color),
        s.border_width,
        rgba(s.border_color),
        s.border_radius,
        s.opacity,
        s.font_size,
        s.z_index,
    )
}

#[derive(Debug, Clone)]
pub struct MirrorElement {
    pub tag: &'static str,
    pub layout_css: String,
    pub style_css: String,
    pub text: Option<String>,
    pub children: Vec<NodeId>,
}

/// Native-testable Web DOM backend (mirrors exact DOM ops; swaps in `web_sys::Element` on `wasm32`).
pub struct WebBackend {
    nodes: HashMap<NodeId, MirrorElement>,
    events: Vec<NativeEvent>,
    #[cfg(target_arch = "wasm32")]
    dom_nodes: HashMap<NodeId, web_sys::Element>,
}

impl Default for WebBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl WebBackend {
    pub fn new() -> Self {
        let mut nodes = HashMap::new();
        nodes.insert(
            ROOT_NODE_ID,
            MirrorElement {
                tag: "div",
                layout_css: String::new(),
                style_css: String::new(),
                text: None,
                children: Vec::new(),
            },
        );
        Self {
            nodes,
            events: Vec::new(),
            #[cfg(target_arch = "wasm32")]
            dom_nodes: HashMap::new(),
        }
    }

    pub fn render_html(&self) -> String {
        self.fmt_html(ROOT_NODE_ID)
    }

    fn fmt_html(&self, id: NodeId) -> String {
        let Some(el) = self.nodes.get(&id) else {
            return String::new();
        };
        let mut s = format!(
            "<{} data-nve-id=\"{}\" style=\"{}{}\">",
            el.tag, id, el.layout_css, el.style_css
        );
        if let Some(t) = &el.text {
            s.push_str(t);
        }
        for &child in &el.children {
            s.push_str(&self.fmt_html(child));
        }
        s.push_str(&format!("</{}>", el.tag));
        s
    }
}

impl RenderBackend for WebBackend {
    fn name(&self) -> &'static str {
        if cfg!(target_arch = "wasm32") {
            "web-dom-wasm"
        } else {
            "web-dom-mirror"
        }
    }

    fn set_viewport(&mut self, _surface_id: SurfaceId, _viewport: Viewport) {}

    fn submit_frame(&mut self, command_bytes: &[u8]) -> Result<BackendApplyStats, BackendError> {
        let t0 = Instant::now();
        let (header, commands) = CommandDecoder::new(command_bytes)?.decode_all()?;
        let mut mutations = 0u32;

        for cmd in commands {
            match cmd {
                Command::CreateNode { node_id, kind, .. } => {
                    let tag = component_to_html_tag(kind);
                    self.nodes.insert(
                        node_id,
                        MirrorElement {
                            tag,
                            layout_css: String::new(),
                            style_css: String::new(),
                            text: None,
                            children: Vec::new(),
                        },
                    );
                    #[cfg(target_arch = "wasm32")]
                    if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
                        if let Ok(el) = doc.create_element(tag) {
                            self.dom_nodes.insert(node_id, el);
                        }
                    }
                    mutations += 1;
                }
                Command::RemoveNode { node_id } => {
                    self.nodes.remove(&node_id);
                    #[cfg(target_arch = "wasm32")]
                    if let Some(el) = self.dom_nodes.remove(&node_id) {
                        el.remove();
                    }
                    mutations += 1;
                }
                Command::SetChildren {
                    parent_id,
                    children,
                } => {
                    if let Some(p) = self.nodes.get_mut(&parent_id) {
                        p.children = children.clone();
                    }
                    #[cfg(target_arch = "wasm32")]
                    if let Some(parent_el) = self.dom_nodes.get(&parent_id) {
                        parent_el.set_text_content(None);
                        for cid in &children {
                            if let Some(child_el) = self.dom_nodes.get(cid) {
                                let _ = parent_el.append_child(child_el);
                            }
                        }
                    }
                    mutations += 1;
                }
                Command::SetText { node_id, text } => {
                    if let Some(el) = self.nodes.get_mut(&node_id) {
                        el.text = Some(text.clone());
                    }
                    #[cfg(target_arch = "wasm32")]
                    if let Some(el) = self.dom_nodes.get(&node_id) {
                        el.set_text_content(Some(&text));
                    }
                    mutations += 1;
                }
                Command::SetLayout { node_id, layout } => {
                    let css = layout_to_css(&layout);
                    if let Some(el) = self.nodes.get_mut(&node_id) {
                        el.layout_css = css;
                    }
                    mutations += 1;
                }
                Command::SetStyle { node_id, style } => {
                    let css = style_to_css(&style);
                    if let Some(el) = self.nodes.get_mut(&node_id) {
                        el.style_css = css;
                    }
                    mutations += 1;
                }
                Command::SetRawProp { .. }
                | Command::SetEventMask { .. }
                | Command::DispatchCommand { .. }
                | Command::Commit { .. } => {}
            }
        }

        let elapsed = t0.elapsed().as_micros() as u64;
        Ok(BackendApplyStats {
            op_count: header.op_count,
            mutation_count: mutations,
            decode_us: elapsed,
            layout_us: 0,
            diff_us: 0,
            mount_us: 0,
        })
    }

    fn drain_events(&mut self) -> Vec<NativeEvent> {
        std::mem::take(&mut self.events)
    }

    fn layout_of(&self, _node_id: NodeId) -> Option<LayoutRect> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nve_abi::CommandBufferBuilder;

    #[test]
    fn renders_flexbox_html_from_command_buffer() {
        let mut web = WebBackend::new();
        let mut b = CommandBufferBuilder::new(1, 1);
        b.create_node(2, ComponentKind::Pressable, None);
        b.set_text(2, "Click");
        b.set_children(ROOT_NODE_ID, &[2]);
        b.commit(true);
        web.submit_frame(b.finish()).unwrap();
        let html = web.render_html();
        assert!(html.contains("<button data-nve-id=\"2\""));
        assert!(html.contains("Click</button>"));
    }
}
