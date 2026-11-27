//! Rendered-tree accessibility audits over a committed [`NodeStore`].

use nve_abi::ColorRgba;
use nve_components::tables::{is_subtree_display_none, TextContent, Typography, Visual};
use nve_store::{NodeId, NodeStore};

use crate::theme::{composite, wcag_contrast};

/// One visible text run below the requested contrast.
#[derive(Debug, Clone, PartialEq)]
pub struct ContrastViolation {
    pub node: NodeId,
    pub text: String,
    pub ratio: f64,
    pub fg: ColorRgba,
    pub bg: ColorRgba,
}

/// Every visible text run whose effective (inherited) color is below `min` WCAG contrast
/// against its effective painted background: translucent ancestor backgrounds are composited
/// over the nearest opaque one (white when none). Runs under a faded (`opacity < 1`, i.e.
/// disabled) or `display: none` ancestor are exempt, as in WCAG 1.4.3.
pub fn text_contrast_violations(store: &NodeStore, min: f64) -> Vec<ContrastViolation> {
    let mut out = Vec::new();
    let mut layers = Vec::new();
    for id in store.live_nodes() {
        let Some(text) = store.try_row::<TextContent>(id) else { continue };
        if text.text().trim().is_empty() || is_subtree_display_none(store, id) {
            continue;
        }
        layers.clear();
        let mut fg = None;
        let mut faded = false;
        let mut opaque = false;
        let mut cur = Some(id);
        while let Some(n) = cur {
            if fg.is_none()
                && let Some(t) = store.try_row::<Typography>(n)
                && t.set() & Typography::SET_COLOR != 0
            {
                fg = Some(ColorRgba::from_u32(t.color().0));
            }
            if let Some(v) = store.try_row::<Visual>(n) {
                faded |= v.opacity() < 1.0;
                let bg = ColorRgba::from_u32(v.background().0);
                if !opaque && bg.a > 0 {
                    layers.push(bg);
                    opaque = bg.a == u8::MAX;
                }
            }
            cur = store.parent(n);
        }
        if faded {
            continue;
        }
        let bg = layers.iter().rev().fold(ColorRgba::WHITE, |under, &over| composite(over, under));
        let fg = composite(fg.unwrap_or(ColorRgba::BLACK), bg);
        let ratio = wcag_contrast(fg, bg);
        if ratio < min {
            out.push(ContrastViolation { node: id, text: text.text().to_owned(), ratio, fg, bg });
        }
    }
    out
}
