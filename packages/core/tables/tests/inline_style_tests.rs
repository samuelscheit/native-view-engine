//! Inline `style="..."` parser coverage: every Layout key the stores, backends and Yoga consume
//! must be reachable from a style string (a missing key is silently dropped, so the layout
//! renders with defaults), and the per-table property bit partitions must stay disjoint.

use nve_tables::tables::{FontStyle, Layout, TextDecoration, Typography};
use nve_tables::{
    STYLE_LAYER_ALL, STYLE_LAYER_LAYOUT, STYLE_LAYER_TYPOGRAPHY, STYLE_LAYER_VISUAL,
    apply_inline_style_layers_to_tx, css_key_bits, prop_bits_to_layers,
};
use nve_store::{Length, NodeStore};

fn layout_after(style: &str) -> (Length, f32, f32, u64) {
    let mut store = NodeStore::new();
    let mut tx = store.tx();
    let id = tx.create();
    let mask = apply_inline_style_layers_to_tx(&mut tx, id, style, STYLE_LAYER_ALL);
    let l = store.row::<Layout>(id);
    (l.flex_basis(), l.flex_grow(), l.flex_shrink(), mask)
}

#[test]
fn flex_basis_is_parsed_in_points_percent_and_both_spellings() {
    let (basis, grow, shrink, mask) = layout_after("flex-grow: 1; flex-shrink: 1; flex-basis: 0%; min-width: 0px");
    assert_eq!(basis, Length::percent(0.0));
    assert_eq!((grow, shrink), (1.0, 1.0));
    let basis_bit = css_key_bits("flex-basis").expect("flex-basis is a known inline-style key");
    assert_eq!(mask & basis_bit, basis_bit, "accepted mask must report flex-basis");

    assert_eq!(layout_after("flexBasis: 240px").0, Length::points(240.0));
    assert_eq!(layout_after("flex-basis: 35%").0, Length::percent(35.0));
    assert!(layout_after("flex-basis: auto").0.is_auto());
    assert!(layout_after("flex-basis: nonsense").0.is_auto(), "invalid values keep the default");
}

#[test]
fn padding_longhands_set_one_side_and_compose_with_shorthands() {
    let mut store = NodeStore::new();
    let mut tx = store.tx();
    let id = tx.create();
    apply_inline_style_layers_to_tx(&mut tx, id, "padding: 4px; padding-bottom: 64px; paddingLeft: 9px", STYLE_LAYER_ALL);
    let l = store.row::<Layout>(id);
    assert_eq!(
        (l.padding_top(), l.padding_right(), l.padding_bottom(), l.padding_left()),
        (Length::points(4.0), Length::points(4.0), Length::points(64.0), Length::points(9.0))
    );
}

#[test]
fn typography_keys_family_style_and_decoration_are_explicit() {
    let mut store = NodeStore::new();
    let mut tx = store.tx();
    let id = tx.create();
    let style = "font-family: ui-monospace, monospace; font-style: italic; text-decoration: underline line-through";
    apply_inline_style_layers_to_tx(&mut tx, id, style, STYLE_LAYER_ALL);
    let t = store.row::<Typography>(id);
    assert_eq!(t.font_family(), "ui-monospace, monospace");
    assert_eq!(t.font_style(), FontStyle::Italic);
    assert_eq!(t.text_decoration(), TextDecoration::UNDERLINE | TextDecoration::LINE_THROUGH);
    assert!(
        t.is_font_family_explicit() && t.is_font_style_explicit() && t.is_text_decoration_explicit(),
        "parsed typography must mark the fields explicit"
    );

    let mut tx = store.tx();
    let other = tx.create();
    apply_inline_style_layers_to_tx(&mut tx, other, "text-decoration: blink; font-style: slanted", STYLE_LAYER_ALL);
    let t = store.row::<Typography>(other);
    assert_eq!(
        (t.text_decoration(), t.font_style()),
        (TextDecoration::empty(), FontStyle::Normal),
        "invalid values keep defaults"
    );
}

#[test]
fn property_bits_are_disjoint_and_route_to_exactly_one_table() {
    let keys = [
        ("width", STYLE_LAYER_LAYOUT), ("flex-shrink", STYLE_LAYER_LAYOUT), ("flex-basis", STYLE_LAYER_LAYOUT),
        ("margin", STYLE_LAYER_LAYOUT), ("background", STYLE_LAYER_VISUAL), ("z-index", STYLE_LAYER_VISUAL),
        ("overflow-y", STYLE_LAYER_VISUAL), ("color", STYLE_LAYER_TYPOGRAPHY), ("text-align", STYLE_LAYER_TYPOGRAPHY),
        ("font-family", STYLE_LAYER_TYPOGRAPHY), ("font-style", STYLE_LAYER_TYPOGRAPHY),
        ("text-decoration", STYLE_LAYER_TYPOGRAPHY),
    ];
    let mut seen = 0u64;
    for (key, layer) in keys {
        let bits = css_key_bits(key).expect("every listed key is a known inline-style key");
        assert_eq!(prop_bits_to_layers(bits), layer, "{key} routes to the wrong style table");
        assert_eq!(seen & bits, 0, "{key} shares a property bit with an earlier key");
        seen |= bits;
    }
}
