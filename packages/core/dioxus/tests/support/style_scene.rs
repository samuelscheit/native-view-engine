#![allow(
    dead_code,
    reason = "each test binary uses a subset of the scene helpers"
)]

//! Declarative `WriteMutations` scene for style-precedence tests: named `view` / `text` nodes
//! mounted under a root, attribute updates grouped into single transactions (one Dioxus frame),
//! and typed reads of the resulting `NodeStore` rows.

use std::collections::HashMap;
use std::rc::Rc;

use dioxus_core::{AttributeValue, ElementId, Template, TemplateNode, WriteMutations};
use nve_abi::SurfaceId;
use nve_tables::tables::{Layout, Typography, Visual};
use nve_dioxus::{NVE_TYPED_STYLE_ATTR, NveMutationWriter, TypedStyle};
use nve_store::{Length, NodeId, Rgba};

macro_rules! tag_template {
    ($tag:literal) => {
        Template {
            roots: &[TemplateNode::Element {
                tag: $tag,
                namespace: None,
                attrs: &[],
                children: &[],
            }],
            node_paths: &[],
            attr_paths: &[],
        }
    };
}

static VIEW: Template = tag_template!("view");
static TEXT: Template = tag_template!("text");

/// Text attribute value, or removal (`AttributeValue::None`) for `None`.
pub fn txt(value: Option<&str>) -> AttributeValue {
    value.map_or(AttributeValue::None, |s| AttributeValue::Text(s.to_owned()))
}

/// Typed-style attribute (`nve_style`).
pub fn typed(style: TypedStyle) -> (&'static str, AttributeValue) {
    (NVE_TYPED_STYLE_ATTR, AttributeValue::Any(Rc::new(style)))
}

pub type Attrs = [(&'static str, AttributeValue)];

pub struct Scene {
    pub writer: NveMutationWriter,
    ids: HashMap<&'static str, ElementId>,
    next: usize,
}

impl Scene {
    /// A scene with a `root` view under the surface root.
    pub fn new() -> Self {
        let mut scene = Self {
            writer: NveMutationWriter::new(SurfaceId::ROOT),
            ids: HashMap::new(),
            next: 1,
        };
        scene.add("root", "view", None);
        scene
    }

    fn el(&self, name: &str) -> ElementId {
        *self.ids.get(name).expect("unknown scene node")
    }

    /// Mounts `name` (`view` or `text`) under `parent` (`root` when `None`).
    pub fn add(&mut self, name: &'static str, tag: &str, parent: Option<&str>) {
        let id = ElementId(self.next);
        self.next += 1;
        let parent_el = parent.map_or(ElementId(0), |p| self.el(p));
        let template = if tag == "text" { TEXT } else { VIEW };
        self.writer.with_transaction(|tx| {
            tx.load_template(template, 0, id);
            tx.append_children(parent_el, 1);
        });
        self.ids.insert(name, id);
    }

    /// Moves the mounted `name` to the end of `parent`'s children (Dioxus `push_root` + append).
    pub fn reparent(&mut self, name: &str, parent: &str) {
        let (child, parent) = (self.el(name), self.el(parent));
        self.writer.with_transaction(|tx| {
            tx.push_root(child);
            tx.append_children(parent, 1);
        });
    }

    /// Removes `name` and its subtree.
    pub fn remove(&mut self, name: &str) {
        let id = self.el(name);
        self.writer.with_transaction(|tx| tx.remove_node(id));
    }

    pub fn is_live(&self, name: &str) -> bool {
        self.writer.resolve_el(self.el(name)).is_some()
    }

    /// Applies attributes to several nodes inside ONE transaction (one frame).
    pub fn frame(&mut self, updates: &[(&str, &Attrs)]) {
        let resolved: Vec<(ElementId, &Attrs)> =
            updates.iter().map(|&(n, a)| (self.el(n), a)).collect();
        self.writer.with_transaction(|tx| {
            for (id, attrs) in resolved {
                for (name, value) in attrs {
                    tx.set_attribute(name, None, value, id);
                }
            }
        });
    }

    /// Applies attributes to one node in its own frame.
    pub fn set(&mut self, name: &str, attrs: &Attrs) {
        self.frame(&[(name, attrs)]);
    }

    pub fn node(&self, name: &str) -> NodeId {
        self.writer.resolve_el(self.el(name)).expect("node bound")
    }

    /// Whether the last frame touched `name` at all.
    pub fn touched(&self, name: &str) -> bool {
        self.writer
            .last_changes()
            .touched
            .contains(&self.node(name))
    }

    pub fn basis(&self, name: &str) -> Length {
        self.layout(name, |l| l.flex_basis())
            .unwrap_or(Length::AUTO)
    }

    /// `(grow, shrink)` with CSS defaults when the node has no `Layout` row.
    pub fn flex_factors(&self, name: &str) -> (f32, f32) {
        self.layout(name, |l| (l.flex_grow(), l.flex_shrink()))
            .unwrap_or((0.0, 1.0))
    }

    pub fn layout<R>(
        &self,
        name: &str,
        f: impl FnOnce(&nve_tables::tables::LayoutRef<'_>) -> R,
    ) -> Option<R> {
        self.writer
            .store()
            .try_row::<Layout>(self.node(name))
            .map(|l| f(&l))
    }

    /// `(top, right, bottom, left)` padding in points (`0` when unset).
    pub fn padding(&self, name: &str) -> [Length; 4] {
        self.layout(name, |l| {
            [
                l.padding_top(),
                l.padding_right(),
                l.padding_bottom(),
                l.padding_left(),
            ]
        })
        .unwrap_or([Length::points(0.0); 4])
    }

    pub fn border_width(&self, name: &str) -> f32 {
        self.writer
            .store()
            .try_row::<Visual>(self.node(name))
            .map_or(0.0, |v| v.border_width())
    }

    pub fn border_color(&self, name: &str) -> Rgba {
        self.writer
            .store()
            .try_row::<Visual>(self.node(name))
            .map_or(Rgba::TRANSPARENT, |v| v.border_color())
    }

    pub fn opacity(&self, name: &str) -> f32 {
        self.writer
            .store()
            .try_row::<Visual>(self.node(name))
            .map_or(1.0, |v| v.opacity())
    }

    /// `Some(color)` when the node's text color is explicit, `None` when inherited.
    pub fn explicit_color(&self, name: &str) -> Option<Rgba> {
        let t = self.writer.store().try_row::<Typography>(self.node(name))?;
        t.is_color_explicit().then(|| t.color())
    }

    /// `Some(px)` when the node's font size is explicit, `None` when inherited.
    pub fn explicit_font_size(&self, name: &str) -> Option<f32> {
        let t = self.writer.store().try_row::<Typography>(self.node(name))?;
        t.is_font_size_explicit().then(|| t.font_size())
    }

    /// The CSS `currentColor` of `name` (nearest explicit color up the tree, else black).
    pub fn text_color(&self, name: &str) -> Rgba {
        nve_tables::effective_text_color(self.writer.store(), self.node(name))
    }
}
