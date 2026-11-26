//! Zero-serialization CSS gradient emission, structural hashing, inline `<svg>` SSR/HTML
//! emission, and live `wasm32` SVG DOM synchronization for [`crate::WebBackend`].

use nve_abi::{ColorRgba, NodeId, RawPropValue, StyleProps};
use nve_components::paint::BackgroundFill;
use nve_components::parse_color;
use nve_components::svg::SvgPicture;

use crate::css::MirrorElement;
use crate::WebBackend;

#[cfg(target_arch = "wasm32")]
const SVG_NS: &str = "http://www.w3.org/2000/svg";

/// Emits compact native CSS `background-image:linear-gradient(...)` / `radial-gradient(...)`
/// (or `background-color:...` for `Solid`) directly from a typed [`BackgroundFill`].
#[inline]
pub fn write_compact_background_fill_into(fill: Option<&BackgroundFill>, out: &mut String) {
    if let Some(fill) = fill {
        fill.write_css_decl_into(false, out);
    }
}

/// Mixes an optional [`BackgroundFill`] into the deterministic FNV-1a64 structural style hash.
#[inline]
pub(crate) fn hash_background_fill(h: u64, fill: Option<&BackgroundFill>) -> u64 {
    match fill {
        Some(f) => f.hash_into(h),
        None => h,
    }
}

/// Resolves the effective optional icon tint color for an SVG node from explicit `tint` raw prop
/// or non-default `style.text_color`.
#[inline]
pub(crate) fn svg_tint_for_node(el: &MirrorElement) -> Option<ColorRgba> {
    if let Some(RawPropValue::String(raw)) = el.raw_props.get("tint")
        && let Some(c) = parse_color(raw)
    {
        return Some(c);
    }
    if el.style.text_color != StyleProps::default().text_color && el.style.text_color.a > 0 {
        return Some(el.style.text_color);
    }
    None
}

/// Writes `viewBox="..."` onto an opening `<svg>` start tag when an [`SvgPicture`] is attached.
pub(crate) fn write_svg_open_attrs_html(el: &MirrorElement, out: &mut String) {
    if el.tag == "svg"
        && let Some(ref pic) = el.svg
    {
        out.push_str(" viewBox=\"");
        pic.write_view_box_into(out);
        out.push('"');
    }
}

/// Writes inline SVG `<path .../>` children (for `<svg>` nodes) or a full inner `<svg>` element
/// (when attached to a container node without element children) into `out` with zero XSS.
pub(crate) fn write_svg_inner_html(el: &MirrorElement, out: &mut String) {
    let Some(ref pic) = el.svg else {
        return;
    };
    if !el.children.is_empty() {
        return;
    }
    let tint = svg_tint_for_node(el);
    if el.tag == "svg" {
        pic.write_svg_children_into(tint, out);
    } else {
        out.push_str("<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"");
        pic.write_view_box_into(out);
        out.push_str("\" style=\"width:100%;height:100%;display:block\">");
        pic.write_svg_children_into(tint, out);
        out.push_str("</svg>");
    }
}

impl WebBackend {
    /// Directly attaches or clears a zero-serialization [`BackgroundFill`] on `id`.
    pub fn set_node_background_fill(&mut self, id: NodeId, fill: Option<BackgroundFill>) {
        let Some(node) = self.nodes.get_mut(&id) else {
            return;
        };
        if node.background_fill != fill {
            node.background_fill = fill;
            self.flush_or_queue_responsive_dirty(vec![id]);
        }
    }

    /// Directly attaches or clears a zero-serialization [`SvgPicture`] on `id`.
    pub fn set_node_svg(&mut self, id: NodeId, svg: Option<SvgPicture>) {
        let Some(node) = self.nodes.get_mut(&id) else {
            return;
        };
        let unchanged = match (&node.svg, &svg) {
            (None, None) => true,
            (Some(a), Some(b)) => a.ptr_eq(b) || a == b,
            _ => false,
        };
        if unchanged {
            return;
        }
        node.svg = svg;
        #[cfg(target_arch = "wasm32")]
        self.sync_dom_svg_node(id);
    }

    /// Synchronizes an attached [`SvgPicture`] onto the live browser DOM using strictly typed
    /// `createElementNS(SVG_NS, ...)` calls (zero `innerHTML`, zero XSS).
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn sync_dom_svg_node(&self, node_id: NodeId) {
        use nve_abi::num::write_f32;
        use nve_components::paint::write_css_color;
        use nve_components::svg::{write_path_data_into, FillRule};

        let (Some(mirror), Some(dom_el)) = (self.nodes.get(&node_id), self.dom_nodes.get(&node_id))
        else {
            return;
        };
        if !mirror.children.is_empty() {
            return;
        }
        let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
            return;
        };

        let Some(ref pic) = mirror.svg else {
            if mirror.tag == "svg" {
                dom_el.set_text_content(None);
                let _ = dom_el.remove_attribute("viewBox");
            } else if let Ok(Some(old)) = dom_el.query_selector("svg[data-nve-inline-svg]") {
                old.remove();
            }
            return;
        };

        let mut vb = String::with_capacity(32);
        pic.write_view_box_into(&mut vb);
        let tint = svg_tint_for_node(mirror);

        let svg_el = if mirror.tag == "svg" {
            let _ = dom_el.set_attribute("viewBox", &vb);
            dom_el.set_text_content(None);
            dom_el.clone()
        } else {
            if let Ok(Some(old)) = dom_el.query_selector("svg[data-nve-inline-svg]") {
                old.remove();
            }
            let Ok(el) = doc.create_element_ns(Some(SVG_NS), "svg") else {
                return;
            };
            let _ = el.set_attribute("data-nve-inline-svg", "1");
            let _ = el.set_attribute("viewBox", &vb);
            let _ = el.set_attribute("style", "width:100%;height:100%;display:block");
            let _ = dom_el.append_child(&el);
            el
        };

        let mut d_buf = String::with_capacity(128);
        let mut color_buf = String::with_capacity(32);
        for shape in pic.shapes.iter() {
            if shape.path.is_empty() {
                continue;
            }
            let Ok(path_el) = doc.create_element_ns(Some(SVG_NS), "path") else {
                continue;
            };
            d_buf.clear();
            write_path_data_into(&shape.path, &mut d_buf);
            let _ = path_el.set_attribute("d", &d_buf);

            let (fill, stroke) = shape.effective_colors(tint);
            match fill {
                Some(c) => {
                    color_buf.clear();
                    write_css_color(c, &mut color_buf);
                    let _ = path_el.set_attribute("fill", &color_buf);
                }
                None => {
                    let _ = path_el.set_attribute("fill", "none");
                }
            }
            if shape.fill_rule == FillRule::EvenOdd {
                let _ = path_el.set_attribute("fill-rule", "evenodd");
            }
            if let Some(sc) = stroke
                && shape.stroke_width.is_finite()
                && shape.stroke_width > 0.0
            {
                color_buf.clear();
                write_css_color(sc, &mut color_buf);
                let _ = path_el.set_attribute("stroke", &color_buf);
                color_buf.clear();
                write_f32(&mut color_buf, shape.stroke_width);
                let _ = path_el.set_attribute("stroke-width", &color_buf);
                let _ = path_el.set_attribute("stroke-linecap", "round");
                let _ = path_el.set_attribute("stroke-linejoin", "round");
            }
            let _ = svg_el.append_child(&path_el);
        }
    }
}
