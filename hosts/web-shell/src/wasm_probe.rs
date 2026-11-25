//! Dev/E2E-only (`dev` feature) semantic HTML5 + `RawStyleProps` hydration probe: mounts a
//! hand-built CommandBuffer as SSR HTML into `#<mount_id>` and hydrates it in place.

use nve_abi::{
    ColorRgba, CommandBufferBuilder, ComponentKind, EdgeInsets, EventMask, LayoutProps,
    RawPropValue, StyleProps, ROOT_NODE_ID,
};
use nve_backend_web::WebBackend;
use nve_engine::RenderBackend;
use wasm_bindgen::prelude::*;

/// Hydrates a semantic HTML5 + `RawStyleProps` (`box-shadow`/`font-weight`/`text-align`)
/// CommandBuffer into `#<mount_id>`, returning the mismatch count (0 on clean hydration).
#[wasm_bindgen]
pub fn run_semantic_style_hydration_probe(mount_id: &str) -> Result<usize, JsValue> {
    let doc = web_sys::window()
        .and_then(|w| w.document())
        .ok_or_else(|| JsValue::from_str("missing document"))?;
    let container = doc
        .get_element_by_id(mount_id)
        .ok_or_else(|| JsValue::from_str(&format!("mount '#{mount_id}' not found")))?;

    let mut b = CommandBufferBuilder::new(99, 1);
    b.create_node(2, ComponentKind::View, None);
    b.set_raw_prop(2, "testID", &RawPropValue::String("probe-card".into()));
    b.set_raw_prop(2, "accessibilityRole", &RawPropValue::String("article".into()));
    b.set_raw_prop(
        2,
        "box-shadow",
        &RawPropValue::String("0 8px 24px rgba(0, 0, 0, 0.45)".into()),
    );
    b.set_layout(
        2,
        &LayoutProps {
            padding: EdgeInsets::all(12.0),
            ..Default::default()
        },
    );
    b.set_style(
        2,
        &StyleProps {
            background: ColorRgba::rgb(17, 24, 39),
            border_radius: 10.0,
            ..Default::default()
        },
    );

    b.create_node(3, ComponentKind::Text, None);
    b.set_raw_prop(3, "testID", &RawPropValue::String("probe-heading".into()));
    b.set_raw_prop(3, "accessibilityRole", &RawPropValue::String("heading".into()));
    b.set_raw_prop(3, "ariaLevel", &RawPropValue::I64(1));
    b.set_raw_prop(3, "font-weight", &RawPropValue::String("700".into()));
    b.set_raw_prop(3, "text-align", &RawPropValue::String("center".into()));
    b.set_style(
        3,
        &StyleProps {
            text_color: ColorRgba::WHITE,
            font_size: 20.0,
            ..Default::default()
        },
    );
    b.set_text(3, "Semantic SSR Heading");

    b.create_node(4, ComponentKind::Pressable, None);
    b.set_raw_prop(4, "testID", &RawPropValue::String("probe-link".into()));
    b.set_raw_prop(4, "accessibilityRole", &RawPropValue::String("link".into()));
    b.set_raw_prop(4, "href", &RawPropValue::String("/docs/ssr".into()));
    b.set_event_mask(4, EventMask::PRESS);
    b.set_text(4, "Docs Link");

    b.set_children(2, &[3, 4]);
    b.set_children(ROOT_NODE_ID, &[2]);
    b.commit(true);
    let frame = b.finish().to_vec();

    // 1. Server render SSR HTML + styles into container if empty.
    if container.query_selector("[data-nve-ssr-hash]").ok().flatten().is_none() {
        let mut server = WebBackend::new();
        server
            .submit_frame(&frame)
            .map_err(|e| JsValue::from_str(&format!("probe server render failed: {e}")))?;
        let ssr = server.render_ssr();
        if let Ok(style_el) = doc.create_element("style") {
            let _ = style_el.set_attribute("data-nve-ssr", "");
            let _ = style_el.set_attribute("data-nve-hashes", &ssr.rule_hashes.join(","));
            style_el.set_text_content(Some(&ssr.style_css));
            if let Some(head) = doc.query_selector("head").ok().flatten() {
                let _ = head.append_child(&style_el);
            }
        }
        container.set_inner_html(&ssr.html);
    }

    // 2. Hydrate in-place and return mismatch count.
    let mut client = WebBackend::hydrate_into(container);
    client
        .submit_frame(&frame)
        .map_err(|e| JsValue::from_str(&format!("probe hydrate failed: {e}")))?;
    let mismatches = client.hydration_report().mismatches.len();
    Ok(mismatches)
}
