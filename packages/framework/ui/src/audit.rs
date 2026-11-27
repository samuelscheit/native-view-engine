//! Rendered-tree accessibility audits over a committed [`NodeStore`].

use nve_abi::ColorRgba;
use nve_components::tables::{
    is_subtree_display_none, Accessibility, AriaTriState, TextContent, Typography, Visual,
};
use nve_store::{NodeId, NodeStore};

use crate::theme::{composite, wcag_contrast};

/// One visible text run or state indicator below the requested contrast.
#[derive(Debug, Clone, PartialEq)]
pub struct ContrastViolation {
    pub node: NodeId,
    /// The text run, or the indicator's state (`checked` / `selected`).
    pub text: String,
    pub ratio: f64,
    pub fg: ColorRgba,
    pub bg: ColorRgba,
}

/// Painted background behind `from` (inclusive) up the ancestor chain: translucent layers are
/// composited over the nearest opaque one (white when none). `None` when any ancestor is faded
/// (`opacity < 1`, i.e. disabled content, exempt as in WCAG 1.4.3 / 1.4.11).
fn effective_background(store: &NodeStore, from: Option<NodeId>, layers: &mut Vec<ColorRgba>) -> Option<ColorRgba> {
    layers.clear();
    let mut opaque = false;
    let mut cur = from;
    while let Some(n) = cur {
        if let Some(v) = store.try_row::<Visual>(n) {
            if v.opacity() < 1.0 {
                return None;
            }
            let bg = ColorRgba::from_u32(v.background().0);
            if !opaque && bg.a > 0 {
                layers.push(bg);
                opaque = bg.a == u8::MAX;
            }
        }
        cur = store.parent(n);
    }
    Some(layers.iter().rev().fold(ColorRgba::WHITE, |under, &over| composite(over, under)))
}

/// Every visible text run whose effective (inherited) color is below `min` WCAG contrast
/// against its effective painted background.
pub fn text_contrast_violations(store: &NodeStore, min: f64) -> Vec<ContrastViolation> {
    let mut out = Vec::new();
    let mut layers = Vec::new();
    for id in store.live_nodes() {
        let Some(text) = store.try_row::<TextContent>(id) else { continue };
        if text.text().trim().is_empty() || is_subtree_display_none(store, id) {
            continue;
        }
        let mut fg = None;
        let mut cur = Some(id);
        while let (None, Some(n)) = (fg, cur) {
            if let Some(t) = store.try_row::<Typography>(n)
                && t.set() & Typography::SET_COLOR != 0
            {
                fg = Some(ColorRgba::from_u32(t.color().0));
            }
            cur = store.parent(n);
        }
        let Some(bg) = effective_background(store, Some(id), &mut layers) else { continue };
        let fg = composite(fg.unwrap_or(ColorRgba::BLACK), bg);
        let ratio = wcag_contrast(fg, bg);
        if ratio < min {
            out.push(ContrastViolation { node: id, text: text.text().to_owned(), ratio, fg, bg });
        }
    }
    out
}

/// Strongest boundary painted by `id` or any descendant (a border when bordered, else an opaque
/// fill), as `(contrast, color, background)` against the background each sits on.
fn strongest_boundary(store: &NodeStore, id: NodeId, layers: &mut Vec<ColorRgba>) -> Option<(f64, ColorRgba, ColorRgba)> {
    let mut best: Option<(f64, ColorRgba, ColorRgba)> = None;
    let mut stack = vec![id];
    while let Some(n) = stack.pop() {
        stack.extend_from_slice(store.children(n));
        let Some(v) = store.try_row::<Visual>(n) else { continue };
        let border = ColorRgba::from_u32(v.border_color().0);
        let fill = ColorRgba::from_u32(v.background().0);
        let mark = if v.border_width() > 0.0 && border.a > 0 { border } else { fill };
        if mark.a == 0 {
            continue;
        }
        let Some(bg) = effective_background(store, store.parent(n), layers) else { continue };
        let mark = composite(mark, bg);
        let ratio = wcag_contrast(mark, bg);
        if best.is_none_or(|(r, ..)| ratio > r) {
            best = Some((ratio, mark, bg));
        }
    }
    best
}

/// WCAG 1.4.11: every visible `checked` / `selected` node must show its state through a boundary
/// (a border when bordered, else a fill, on the node or its indicator descendant such as a
/// checkbox box or switch track) of at least `min` contrast against the background it sits on.
pub fn indicator_contrast_violations(store: &NodeStore, min: f64) -> Vec<ContrastViolation> {
    let mut out = Vec::new();
    let mut layers = Vec::new();
    for id in store.live_nodes() {
        let Some(a) = store.try_row::<Accessibility>(id) else { continue };
        let state = match (a.checked(), a.selected()) {
            (AriaTriState::True | AriaTriState::Mixed, _) => "checked",
            (_, AriaTriState::True) => "selected",
            _ => continue,
        };
        if is_subtree_display_none(store, id) {
            continue;
        }
        // Faded (disabled) indicators are exempt.
        let Some(page) = effective_background(store, Some(id), &mut layers) else { continue };
        let (ratio, fg, bg) = strongest_boundary(store, id, &mut layers).unwrap_or((1.0, page, page));
        if ratio < min {
            out.push(ContrastViolation { node: id, text: state.to_owned(), ratio, fg, bg });
        }
    }
    out
}
