#![cfg(feature = "ssr")]
//! `flex-basis` raw style routing (incl. the basis of a CSS `flex` shorthand): stored, hashed
//! into distinct structural classes, emitted after the `.nve{flex:0 1 auto}` reset, and never
//! leaked as an HTML attribute.

use nve_abi::{CommandBufferBuilder, ComponentKind, RawPropValue, ROOT_NODE_ID};
use nve_backend_web::{is_raw_style_prop, RawStyleProps, WebBackend};
use nve_engine::RenderBackend;

#[test]
fn flex_basis_routes_hashes_and_emits_without_html_attribute() {
    for key in ["flex-basis", "flexBasis", "flex_basis"] {
        assert!(is_raw_style_prop(key), "{key}");
    }
    let mut raw = RawStyleProps::default();
    assert!(raw.apply_prop("flexBasis", &RawPropValue::String(" 280px ".into())));
    assert_eq!(raw.effective_flex_basis(), Some("280px"));
    assert!(raw.apply_prop("flex-basis", &RawPropValue::String("auto".into())));
    assert_eq!(raw.effective_flex_basis(), None, "auto is the reset default");
    assert!(raw.apply_prop("flex-basis", &RawPropValue::String("1px;color:red".into())));
    assert_eq!(raw.flex_basis, None, "unsafe values clear the slot");

    let mut web = WebBackend::new();
    let mut b = CommandBufferBuilder::new(1, 1);
    for (id, basis) in [(2, "280px"), (3, "110px"), (4, "0%")] {
        b.create_node(id, ComponentKind::View, None);
        b.set_raw_prop(id, "flex-basis", &RawPropValue::String(basis.into()));
    }
    b.set_children(ROOT_NODE_ID, &[2, 3, 4]);
    b.commit(true);
    web.submit_frame(b.finish()).unwrap();

    let ssr = web.render_ssr();
    for basis in ["280px", "110px", "0%"] {
        assert!(
            ssr.style_css.contains(&format!("flex-basis:{basis};")),
            "each distinct basis needs its own structural rule: {}",
            ssr.style_css
        );
    }
    assert!(!ssr.html.contains("flex-basis="), "{}", ssr.html);
    assert!(!ssr.html.contains("flexBasis="), "{}", ssr.html);
}
