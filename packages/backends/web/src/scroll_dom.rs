//! `wasm32` 120 Hz compositor-only CSSOM fast lane (`apply_compositor_properties_live`) and
//! exact controlled scroll sync (`sync_requested_scroll`).

use nve_store::{NodeId, NodeStore};
use nve_tables::tables::{Scroll, Transform, Visual};
use wasm_bindgen::JsCast;

use crate::WebBackend;

impl WebBackend {
    #[inline]
    pub(crate) fn clear_scroll_seq(&mut self, id: NodeId) {
        let idx = id.index();
        if idx < self.last_requested_scroll_seq.len() {
            self.last_requested_scroll_seq[idx] = 0;
        }
        self.programmatic_scroll_guard.borrow_mut().remove(&id);
    }

    /// Reconciles declarative `Scroll::requested_scroll_{x,y,seq}` to live DOM `scrollLeft` /
    /// `scrollTop` without clobbering user scroll or echoing feedback loops.
    pub(crate) fn sync_requested_scroll(
        &mut self,
        store: &NodeStore,
        id: NodeId,
        el: &web_sys::Element,
        defer_to_step6: bool,
    ) {
        let Some(sc) = store.try_row::<Scroll>(id) else {
            self.clear_scroll_seq(id);
            return;
        };
        let seq = sc.requested_scroll_seq();
        if seq == 0 {
            return;
        }
        let idx = id.index();
        let prev = self.last_requested_scroll_seq.get(idx).copied().unwrap_or(0);
        if seq == prev {
            return;
        }
        if defer_to_step6 || !el.is_connected() {
            if !self.pending_scroll_sync.contains(&id) {
                self.pending_scroll_sync.push(id);
            }
            return;
        }
        if idx >= self.last_requested_scroll_seq.len() {
            self.last_requested_scroll_seq.resize(idx + 1, 0);
        }
        self.last_requested_scroll_seq[idx] = seq;

        let cur_x = el.scroll_left();
        let cur_y = el.scroll_top();
        let mut wrote = false;
        if let Some(rx) = sc.requested_scroll_x().filter(|v| v.is_finite()) {
            let wx = rx.max(0.0).round() as i32;
            if wx != cur_x {
                el.set_scroll_left(wx);
                wrote = true;
            }
        }
        if let Some(ry) = sc.requested_scroll_y().filter(|v| v.is_finite()) {
            let wy = ry.max(0.0).round() as i32;
            if wy != cur_y {
                el.set_scroll_top(wy);
                wrote = true;
            }
        }
        if wrote {
            let after = (el.scroll_left(), el.scroll_top());
            self.programmatic_scroll_guard.borrow_mut().insert(id, after);
        }
    }
}

/// Applies `effect::COMPOSITE` properties (`opacity`, `z-index`, `transform`, `transform-origin`)
/// directly via CSSOM `style.setProperty`/`removeProperty` so `inline_fallback`, plugin inline CSS
/// (e.g. `backdrop-filter`), and `patch_css` are never clobbered on the 120 Hz fast lane.
pub(crate) fn apply_compositor_properties_live(
    store: &NodeStore,
    id: NodeId,
    el: &web_sys::Element,
) {
    let style = if let Some(html_el) = el.dyn_ref::<web_sys::HtmlElement>() {
        html_el.style()
    } else if let Some(svg_el) = el.dyn_ref::<web_sys::SvgElement>() {
        svg_el.style()
    } else {
        return;
    };
    if let Some(v) = store.try_row::<Visual>(id) {
        if (v.opacity() - 1.0).abs() > 1e-4 {
            let mut buf = String::with_capacity(8);
            crate::css::write_compact_num(v.opacity().clamp(0.0, 1.0), &mut buf);
            let _ = style.set_property("opacity", &buf);
        } else {
            let _ = style.remove_property("opacity");
        }
        if v.z_index() != 0 {
            let _ = style.set_property("z-index", &v.z_index().to_string());
        } else {
            let _ = style.remove_property("z-index");
        }
    } else {
        let _ = style.remove_property("opacity");
        let _ = style.remove_property("z-index");
    }

    if let Some(tf) = store.try_row::<Transform>(id) {
        let (tx, ty) = (tf.translate_x(), tf.translate_y());
        let (sx, sy) = (tf.scale_x(), tf.scale_y());
        let rot = tf.rotate_deg();
        if tx != 0.0
            || ty != 0.0
            || (sx - 1.0).abs() > 1e-4
            || (sy - 1.0).abs() > 1e-4
            || rot != 0.0
        {
            let mut val = String::with_capacity(48);
            let mut first = true;
            if tx != 0.0 || ty != 0.0 {
                val.push_str("translate(");
                crate::css::write_px(tx, &mut val);
                val.push(',');
                crate::css::write_px(ty, &mut val);
                val.push(')');
                first = false;
            }
            if (sx - 1.0).abs() > 1e-4 || (sy - 1.0).abs() > 1e-4 {
                if !first {
                    val.push(' ');
                }
                val.push_str("scale(");
                crate::css::write_compact_num(sx, &mut val);
                val.push(',');
                crate::css::write_compact_num(sy, &mut val);
                val.push(')');
                first = false;
            }
            if rot != 0.0 {
                if !first {
                    val.push(' ');
                }
                val.push_str("rotate(");
                crate::css::write_compact_num(rot, &mut val);
                val.push_str("deg)");
            }
            let _ = style.set_property("transform", &val);
        } else {
            let _ = style.remove_property("transform");
        }
        if (tf.origin_x() - 0.5).abs() > 1e-4 || (tf.origin_y() - 0.5).abs() > 1e-4 {
            let mut orig = String::with_capacity(20);
            crate::css::write_compact_num(tf.origin_x() * 100.0, &mut orig);
            orig.push_str("% ");
            crate::css::write_compact_num(tf.origin_y() * 100.0, &mut orig);
            orig.push('%');
            let _ = style.set_property("transform-origin", &orig);
        } else {
            let _ = style.remove_property("transform-origin");
        }
    } else {
        let _ = style.remove_property("transform");
        let _ = style.remove_property("transform-origin");
    }
}
