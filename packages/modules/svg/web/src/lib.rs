//! Web DOM & SSR adapter for `nve-svg` (`WebTableSystem`).

#![cfg_attr(doctest, doc = include_str!("../README.md"))]

use nve_backend_web::{WebSystemCommitCtx, WebTableSystem};
use nve_store::{NodeId, NodeStore, NveTable, Rgba, TableSlot};
use nve_svg::{ColorRgba, Svg, SvgPicture};

/// `WebTableSystem` plugin for [`Svg`] (`nve.svg`).
#[derive(Debug, Default, Clone, Copy)]
pub struct SvgWebSystem;

impl SvgWebSystem {
    #[inline]
    fn resolved_tint(tint: Rgba) -> Option<ColorRgba> {
        (tint != Rgba::TRANSPARENT).then(|| ColorRgba::from_u32(tint.0))
    }

    fn write_view_box_and_mirror(store: &NodeStore, node: NodeId) -> Option<(String, bool)> {
        let row = store.try_row::<Svg>(node)?;
        let pic = store.resource::<SvgPicture>(row.picture())?;
        let mut vb = String::with_capacity(24);
        pic.write_view_box_into(&mut vb);
        Some((vb, row.mirror_directional()))
    }

    fn write_inner_into(store: &NodeStore, node: NodeId, out: &mut String) -> bool {
        let Some(row) = store.try_row::<Svg>(node) else {
            return false;
        };
        let Some(pic) = store.resource::<SvgPicture>(row.picture()) else {
            return false;
        };
        pic.write_svg_children_into(Self::resolved_tint(row.tint()), out);
        true
    }
}

impl WebTableSystem for SvgWebSystem {
    fn slot(&self) -> TableSlot {
        Svg::slot()
    }

    fn priority(&self) -> u8 {
        120
    }

    fn tag_name<'a>(&self, _store: &'a NodeStore, _node: NodeId) -> Option<&'a str> {
        Some("svg")
    }

    fn namespace(&self) -> Option<&'static str> {
        Some("http://www.w3.org/2000/svg")
    }

    fn write_ssr_attrs(&self, store: &NodeStore, node: NodeId, out: &mut String) {
        if let Some((vb, mirror)) = Self::write_view_box_and_mirror(store, node) {
            out.push_str(" viewBox=\"");
            out.push_str(&vb);
            out.push('"');
            if mirror {
                out.push_str(" data-nve-mirror=\"directional\"");
            }
        }
    }

    fn write_ssr_inner(&self, store: &NodeStore, node: NodeId, out: &mut String) -> bool {
        Self::write_inner_into(store, node, out)
    }

    fn commit_node(
        &self,
        ctx: &mut WebSystemCommitCtx<'_>,
        store: &NodeStore,
        node: NodeId,
        _dirty_words: u32,
    ) {
        if let Some((vb, mirror)) = Self::write_view_box_and_mirror(store, node) {
            ctx.set_attr(node, "viewBox", &vb);
            if mirror {
                ctx.set_attr(node, "data-nve-mirror", "directional");
            } else {
                ctx.remove_attr(node, "data-nve-mirror");
            }
            let mut inner = String::new();
            if Self::write_inner_into(store, node, &mut inner) {
                ctx.set_inner_html(node, inner);
            }
        } else {
            ctx.remove_attr(node, "viewBox");
            ctx.remove_attr(node, "data-nve-mirror");
            ctx.clear_inner_html(node);
        }
    }
}
