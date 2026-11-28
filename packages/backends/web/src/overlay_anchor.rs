//! Cross-browser CSS Anchor Positioning (`anchor-name`, `position-anchor`, `position-area`,
//! `position-try-fallbacks`) for `nve.overlay` nodes plus a cleanup-safe `getBoundingClientRect()`
//! and [`nve_layout::overlay::compute_overlay_position`] fallback on `wasm32` when native CSS
//! Anchor Positioning is unsupported or `OverlayFlags::AUTO_SIZE` is active.

use nve_store::{NodeId, NodeStore, NveTable, TableStorage};
use nve_tables::tables::{DebugId, Overlay, OverlayFlags, OverlayLayer, OverlayPlacement, Side};

use crate::css::write_px;

/// Sanitizes an arbitrary DOM `id` into a valid CSS `<dashed-ident>` (`--nve-a-<ident>`).
pub fn write_anchor_ident(raw_id: &str, out: &mut String) {
    out.push_str("--nve-a-");
    for ch in raw_id.chars().take(64) {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
}

/// Maps [`OverlayPlacement`] to its exact CSS Anchor Positioning `position-area` value.
#[inline]
pub const fn css_position_area(placement: OverlayPlacement) -> &'static str {
    match placement {
        OverlayPlacement::Top => "top center",
        OverlayPlacement::TopStart => "top span-right",
        OverlayPlacement::TopEnd => "top span-left",
        OverlayPlacement::Bottom => "bottom center",
        OverlayPlacement::BottomStart => "bottom span-right",
        OverlayPlacement::BottomEnd => "bottom span-left",
        OverlayPlacement::Left => "left center",
        OverlayPlacement::LeftStart => "left span-bottom",
        OverlayPlacement::LeftEnd => "left span-top",
        OverlayPlacement::Right => "right center",
        OverlayPlacement::RightStart => "right span-bottom",
        OverlayPlacement::RightEnd => "right span-top",
    }
}

/// Returns `true` when `dom_id` is referenced by at least one live [`Overlay::anchor_id`] in
/// `store` (O(1) zero-work fast path when no `Overlay` rows exist).
pub fn is_referenced_overlay_anchor(store: &NodeStore, dom_id: &str) -> bool {
    if dom_id.is_empty() {
        return false;
    }
    let Some(TableStorage {
        owners, row_count, ..
    }) = store.table_storage(Overlay::slot())
    else {
        return false;
    };
    if row_count == 0 {
        return false;
    }
    owners.iter().take(row_count).any(|&oid| {
        !oid.is_null()
            && store.is_live(oid)
            && store
                .try_row::<Overlay>(oid)
                .is_some_and(|ov| ov.anchor_id() == dom_id)
    })
}

/// Emits CSS Anchor Positioning declarations (`anchor-name` on referenced anchors; `position:fixed`,
/// `position-anchor`, `position-area`, `position-try-fallbacks`, anchor-facing offset margin, or
/// closed hiding on [`Overlay`] nodes).
pub fn write_overlay_anchor_css(store: &NodeStore, id: NodeId, out: &mut String) {
    if let Some(dbg) = store.try_row::<DebugId>(id) {
        let dom_id = dbg.id();
        if is_referenced_overlay_anchor(store, dom_id) {
            out.push_str("anchor-name:");
            write_anchor_ident(dom_id, out);
            out.push(';');
        }
    }
    let Some(ov) = store.try_row::<Overlay>(id) else {
        return;
    };
    let opts = ov.options();
    if !opts.contains(OverlayFlags::OPEN) {
        out.push_str("display:none;");
        return;
    }
    out.push_str("position:fixed;");
    let anchor_id = ov.anchor_id();
    if !anchor_id.is_empty() {
        out.push_str("position-anchor:");
        write_anchor_ident(anchor_id, out);
        out.push_str(";position-area:");
        out.push_str(css_position_area(ov.placement()));
        out.push(';');
        let off = ov.offset_px();
        if off.is_finite() && off != 0.0 {
            let prop = match ov.placement().side() {
                Side::Top => "margin-bottom:",
                Side::Bottom => "margin-top:",
                Side::Left => "margin-right:",
                Side::Right => "margin-left:",
            };
            out.push_str(prop);
            write_px(off, out);
            out.push(';');
        }
        if opts.contains(OverlayFlags::FLIP) {
            out.push_str("position-try-fallbacks:flip-block,flip-inline,flip-block flip-inline;");
        }
    } else if ov.layer() == OverlayLayer::Modal {
        out.push_str("margin:auto;");
    }
}

/// Pure decision for whether an open anchored [`Overlay`] requires the JS
/// `getBoundingClientRect()` + `compute_overlay_position` positioning pass.
#[inline]
pub const fn needs_js_anchor_fallback(
    supports_css_anchor: bool,
    has_anchor: bool,
    opts: OverlayFlags,
) -> bool {
    has_anchor
        && opts.contains(OverlayFlags::OPEN)
        && (!supports_css_anchor || opts.contains(OverlayFlags::AUTO_SIZE))
}

#[cfg(target_arch = "wasm32")]
pub(crate) use live::OverlayAnchorFallbackManager;

#[cfg(target_arch = "wasm32")]
mod live {
    use std::cell::RefCell;
    use std::rc::Rc;

    use nve_abi::hash::HashMap;
    use nve_layout::overlay::{
        PositionOptions, Rect, Size, Viewport as OverlayViewport, compute_overlay_position,
    };
    use nve_store::{NodeId, NodeStore};
    use nve_tables::tables::{Overlay, OverlayFlags};
    use wasm_bindgen::JsCast;
    use wasm_bindgen::closure::Closure;
    use wasm_bindgen::prelude::wasm_bindgen;

    use super::needs_js_anchor_fallback;
    use crate::css::write_px;
    use crate::events::DomListenerGuard;

    #[wasm_bindgen(inline_js = "
export function nve_supports_css_anchor() {
  return typeof CSS !== 'undefined'
    && typeof CSS.supports === 'function'
    && CSS.supports('anchor-name', '--a');
}
")]
    extern "C" {
        fn nve_supports_css_anchor() -> bool;
    }

    #[derive(Clone)]
    struct FallbackEntry {
        el: web_sys::Element,
        anchor_id: String,
        opts: PositionOptions,
    }

    /// Owns active `getBoundingClientRect()` fallback registrations and a single pair of shared
    /// capture-scroll + resize listeners that detach automatically when empty or dropped.
    pub(crate) struct OverlayAnchorFallbackManager {
        supports_css_anchor: bool,
        entries: Rc<RefCell<HashMap<NodeId, FallbackEntry>>>,
        listeners: Option<[DomListenerGuard; 2]>,
    }

    impl Default for OverlayAnchorFallbackManager {
        fn default() -> Self {
            Self {
                supports_css_anchor: nve_supports_css_anchor(),
                entries: Rc::new(RefCell::new(HashMap::default())),
                listeners: None,
            }
        }
    }

    impl OverlayAnchorFallbackManager {
        pub(crate) fn remove(&mut self, id: NodeId) {
            let empty = {
                let mut map = self.entries.borrow_mut();
                map.remove(&id);
                map.is_empty()
            };
            if empty {
                self.listeners = None;
            }
        }

        pub(crate) fn clear(&mut self) {
            self.entries.borrow_mut().clear();
            self.listeners = None;
        }

        pub(crate) fn sync_node(&mut self, store: &NodeStore, id: NodeId, el: &web_sys::Element) {
            let Some(ov) = store.try_row::<Overlay>(id) else {
                self.remove(id);
                return;
            };
            let opts = ov.options();
            let anchor_id = ov.anchor_id();
            if !needs_js_anchor_fallback(self.supports_css_anchor, !anchor_id.is_empty(), opts) {
                self.remove(id);
                return;
            }
            let entry = FallbackEntry {
                el: el.clone(),
                anchor_id: anchor_id.to_owned(),
                opts: PositionOptions {
                    placement: ov.placement(),
                    offset: ov.offset_px(),
                    viewport_padding: ov.viewport_padding_px(),
                    flip: opts.contains(OverlayFlags::FLIP),
                    shift: opts.contains(OverlayFlags::SHIFT),
                    auto_size: opts.contains(OverlayFlags::AUTO_SIZE),
                    fallback_placements: Vec::new(),
                },
            };
            reposition_entry(&entry);
            self.entries.borrow_mut().insert(id, entry);
            self.ensure_listeners();
        }

        pub(crate) fn reposition_all(&self) {
            let entries = self.entries.borrow();
            for entry in entries.values() {
                reposition_entry(entry);
            }
        }

        fn ensure_listeners(&mut self) {
            if self.listeners.is_some() {
                return;
            }
            let Some(win) = web_sys::window() else {
                return;
            };
            let target: web_sys::EventTarget = win.into();
            let make = |name: &'static str, capture: bool| {
                let entries = Rc::clone(&self.entries);
                let cb = Closure::<dyn Fn(web_sys::Event)>::new(move |_| {
                    if let Ok(map) = entries.try_borrow() {
                        for entry in map.values() {
                            reposition_entry(entry);
                        }
                    }
                });
                DomListenerGuard::attach(target.clone(), name, capture, cb)
            };
            self.listeners = Some([make("scroll", true), make("resize", false)]);
        }
    }

    fn reposition_entry(entry: &FallbackEntry) {
        if !entry.el.is_connected() {
            return;
        }
        let Some(win) = web_sys::window() else {
            return;
        };
        let Some(doc) = win.document() else {
            return;
        };
        let Some(anchor_el) = doc.get_element_by_id(&entry.anchor_id) else {
            return;
        };
        let Some(html_el) = entry.el.dyn_ref::<web_sys::HtmlElement>() else {
            return;
        };
        let ar = anchor_el.get_bounding_client_rect();
        let or = entry.el.get_bounding_client_rect();
        let vp_w = win.inner_width().ok().and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
        let vp_h = win.inner_height().ok().and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
        let pos = compute_overlay_position(
            Rect::new(ar.left() as f32, ar.top() as f32, ar.width() as f32, ar.height() as f32),
            Size::new(or.width() as f32, or.height() as f32),
            OverlayViewport::new(vp_w, vp_h),
            &entry.opts,
        );
        let style = html_el.style();
        let _ = style.set_property("position", "fixed");
        let _ = style.set_property("position-area", "none");
        let _ = style.set_property("margin", "0");
        let mut buf = String::with_capacity(16);
        write_px(pos.x, &mut buf);
        let _ = style.set_property("left", &buf);
        buf.clear();
        write_px(pos.y, &mut buf);
        let _ = style.set_property("top", &buf);
        if let Some(mw) = pos.max_width {
            buf.clear();
            write_px(mw, &mut buf);
            let _ = style.set_property("max-width", &buf);
        }
        if let Some(mh) = pos.max_height {
            buf.clear();
            write_px(mh, &mut buf);
            let _ = style.set_property("max-height", &buf);
        }
    }
}
