//! Web DOM & SSR adapter for `nve-blur` (`WebTableSystem`).

#![cfg_attr(doctest, doc = include_str!("../README.md"))]

use core::fmt::Write as _;

use nve_backend_web::{WebSystemCommitCtx, WebTableSystem};
use nve_blur::BlurSurface;
use nve_store::{NodeId, NodeStore, NveTable, TableSlot};

/// `WebTableSystem` plugin for [`BlurSurface`] (`nve.blur_surface`).
#[derive(Debug, Default, Clone, Copy)]
pub struct BlurWebSystem;

impl BlurWebSystem {
    fn format_css(store: &NodeStore, node: NodeId, out: &mut String) {
        let Some(row) = store.try_row::<BlurSurface>(node) else {
            return;
        };
        let raw = row.intensity();
        let clamped = if raw.is_finite() {
            raw.clamp(0.0, 1.0)
        } else {
            1.0
        };
        let px = (row.material().css_blur_px() * clamped).max(0.0);
        let _ = write!(out, "backdrop-filter:blur({px}px);-webkit-backdrop-filter:blur({px}px);");
        if let Some(t) = row.tint() {
            let a = f32::from(t.a()) / 255.0;
            let _ = write!(out, "background-color:rgba({},{},{},{a});", t.r(), t.g(), t.b());
        }
    }
}

impl WebTableSystem for BlurWebSystem {
    fn slot(&self) -> TableSlot {
        BlurSurface::slot()
    }

    fn write_inline_css(&self, store: &NodeStore, node: NodeId, out: &mut String) {
        Self::format_css(store, node, out);
    }

    fn commit_node(
        &self,
        ctx: &mut WebSystemCommitCtx<'_>,
        store: &NodeStore,
        node: NodeId,
        _dirty_words: u32,
    ) {
        let mut css = String::new();
        Self::format_css(store, node, &mut css);
        ctx.set_inline_css(node, css);
    }
}
