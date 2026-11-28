//! Verification tests for:
//! 1. Full 21-field Web ARIA parity (`visit_accessibility_attrs` == SSR HTML).
//! 2. Declarative pointer capture (`data-nve-capture="primary"`, `WebEventState::capture_count`,
//!    CSS touch-action/user-select reset) & disabled cursor CSS reset.
//! 3. CSS Anchor Positioning (`anchor-name`, `position-anchor`, `position-area`,
//!    `position-try-fallbacks`, UA popover/dialog reset, and JS fallback gating).

use std::collections::BTreeMap;

use nve_backend_web::overlay_anchor::{needs_js_anchor_fallback, write_overlay_anchor_css};
use nve_backend_web::{
    visit_accessibility_attrs, WebBackend, WebEventState, ALL_ACCESSIBILITY_ATTRS,
    NVE_BASE_RESET_CSS,
};
use nve_engine::StoreBackend;
use nve_store::{ChangeSet, NodeId, NodeStore};
use nve_tables::tables::{
    register_core_tables, Accessibility, AriaAutoComplete, AriaLive, AriaOrientation,
    AriaRelations, AriaRole, AriaTriState, DebugId, Interaction, Overlay, OverlayFlags,
    OverlayLayer, OverlayPlacement, PointerCaptureMode,
};

#[test]
fn all_21_accessibility_fields_emit_identically_in_visitor_and_ssr() {
    register_core_tables();
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let target = {
        let mut tx = store.tx();
        let node = tx.create();
        tx.edit::<Accessibility>(node)
            .role(Some(AriaRole::ComboBox))
            .level(Some(2))
            .modal(true)
            .live(Some(AriaLive::Assertive))
            .orientation(Some(AriaOrientation::Vertical))
            .autocomplete(Some(AriaAutoComplete::List))
            .checked(Some(AriaTriState::Mixed))
            .selected(Some(AriaTriState::True))
            .expanded(Some(AriaTriState::False))
            .pressed(Some(AriaTriState::True))
            .hidden(true)
            .invalid(true)
            .multiselectable(true)
            .busy(true)
            .label("Country")
            .hint("Choose a country")
            .value_text("50 percent")
            .value_now(Some(50.0))
            .value_min(Some(0.0))
            .value_max(Some(100.0));
        tx.set_resource(
            node,
            Accessibility::RELATIONS,
            Some(AriaRelations {
                controls: "panel-1".into(),
                labelled_by: "heading-1".into(),
                described_by: "hint-1".into(),
                active_descendant: "opt-3".into(),
            }),
        );
        tx.set_children(NodeId::ROOT, &[node]);
        tx.commit_into(&mut changes);
        node
    };

    let mut visited = BTreeMap::new();
    visit_accessibility_attrs(&store, target, |name, val| {
        visited.insert(name, val.to_owned());
    });

    // Every single one of the 24 HTML attributes backed by the 21 `Accessibility` fields must be present.
    assert_eq!(visited.len(), ALL_ACCESSIBILITY_ATTRS.len());
    let expected: &[(&str, &str)] = &[
        ("role", "combobox"),
        ("aria-level", "2"),
        ("aria-modal", "true"),
        ("aria-live", "assertive"),
        ("aria-orientation", "vertical"),
        ("aria-autocomplete", "list"),
        ("aria-checked", "mixed"),
        ("aria-selected", "true"),
        ("aria-expanded", "false"),
        ("aria-pressed", "true"),
        ("aria-hidden", "true"),
        ("aria-invalid", "true"),
        ("aria-multiselectable", "true"),
        ("aria-busy", "true"),
        ("aria-label", "Country"),
        ("aria-description", "Choose a country"),
        ("aria-valuetext", "50 percent"),
        ("aria-valuenow", "50"),
        ("aria-valuemin", "0"),
        ("aria-valuemax", "100"),
        ("aria-controls", "panel-1"),
        ("aria-labelledby", "heading-1"),
        ("aria-describedby", "hint-1"),
        ("aria-activedescendant", "opt-3"),
    ];
    for &(k, v) in expected {
        assert_eq!(
            visited.get(k).map(String::as_str),
            Some(v),
            "visitor attribute mismatch for {k}"
        );
    }

    // SSR output must emit the exact same 24 attributes verbatim.
    let mut web = WebBackend::new();
    web.commit(&store, &changes).unwrap();
    let ssr = web.renderer().render_ssr(&store, NodeId::ROOT);
    for &(k, v) in expected {
        let needle = format!("{k}=\"{v}\"");
        assert!(
            ssr.contains(&needle),
            "SSR missing `{needle}` in output: {ssr}"
        );
    }
}

#[test]
fn base_reset_css_pointer_capture_and_css_anchor_positioning_parity() {
    assert!(NVE_BASE_RESET_CSS.contains(
        ":where(button.nve:disabled,.nve[aria-disabled=\"true\"]){cursor:not-allowed}"
    ));
    assert!(NVE_BASE_RESET_CSS.contains(
        ":where([data-nve-capture=\"primary\"]){touch-action:none;user-select:none}"
    ));
    assert!(NVE_BASE_RESET_CSS.contains(":where(dialog.nve,.nve[popover]){"));

    register_core_tables();
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let (slider, unreferenced, anchor, overlay) = {
        let mut tx = store.tx();
        let slider = tx.create();
        tx.edit::<Interaction>(slider).capture_mode(PointerCaptureMode::OnPrimaryDown);

        let unreferenced = tx.create();
        tx.edit::<DebugId>(unreferenced).id("plain-heading");

        let anchor = tx.create();
        tx.edit::<DebugId>(anchor).id("menu:trigger/1");

        let overlay = tx.create();
        tx.edit::<Overlay>(overlay)
            .layer(OverlayLayer::Menu)
            .anchor_id("menu:trigger/1")
            .placement(OverlayPlacement::BottomStart)
            .offset_px(6.0)
            .options(OverlayFlags::OPEN | OverlayFlags::FLIP | OverlayFlags::SHIFT);

        tx.set_children(NodeId::ROOT, &[slider, unreferenced, anchor, overlay]);
        tx.commit_into(&mut changes);
        (slider, unreferenced, anchor, overlay)
    };

    let mut web = WebBackend::new();
    web.commit(&store, &changes).unwrap();
    let ssr = web.renderer().render_ssr(&store, NodeId::ROOT);
    assert!(ssr.contains("data-nve-capture=\"primary\""));
    assert!(ssr.contains("data-nve-overlay=\"open\""));

    let mut unref_css = String::new();
    write_overlay_anchor_css(&store, unreferenced, &mut unref_css);
    assert!(
        unref_css.is_empty(),
        "unreferenced DebugId must not emit spurious anchor-name: {unref_css}"
    );

    let mut anchor_css = String::new();
    write_overlay_anchor_css(&store, anchor, &mut anchor_css);
    assert_eq!(anchor_css, "anchor-name:--nve-a-menu_trigger_1;");

    let mut ov_css = String::new();
    write_overlay_anchor_css(&store, overlay, &mut ov_css);
    assert!(ov_css.contains("position:fixed;"));
    assert!(ov_css.contains("position-anchor:--nve-a-menu_trigger_1;"));
    assert!(ov_css.contains("position-area:bottom span-right;"));
    assert!(ov_css.contains("margin-top:6px;"));
    assert!(ov_css.contains("position-try-fallbacks:flip-block,flip-inline,flip-block flip-inline;"));

    assert!(!needs_js_anchor_fallback(true, true, OverlayFlags::OPEN | OverlayFlags::FLIP));
    assert!(needs_js_anchor_fallback(false, true, OverlayFlags::OPEN));
    assert!(needs_js_anchor_fallback(true, true, OverlayFlags::OPEN | OverlayFlags::AUTO_SIZE));
    assert!(!needs_js_anchor_fallback(false, true, OverlayFlags::NONE));
    let _ = slider;
}

#[test]
fn disabled_capture_is_excluded_and_accessibility_escaping_clearing_and_root_match() {
    register_core_tables();
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let (disabled_slider, escaped_node) = {
        let mut tx = store.tx();
        // Root Accessibility must be suppressed in SSR (aligned with live DOM).
        tx.edit::<Accessibility>(NodeId::ROOT)
            .role(Some(AriaRole::Navigation))
            .label("root-nav");

        let disabled_slider = tx.create();
        tx.edit::<Interaction>(disabled_slider)
            .capture_mode(PointerCaptureMode::OnPrimaryDown)
            .disabled(true);

        let escaped = tx.create();
        tx.edit::<Accessibility>(escaped)
            .role(Some(AriaRole::Button))
            .label("Save & \"Close\" <now>")
            .hint("Press <Enter> & \"confirm\"")
            .value_text("1 < 2 & \"3\"");
        tx.set_resource(
            escaped,
            Accessibility::RELATIONS,
            Some(AriaRelations {
                controls: "p&1\"<x>".into(),
                labelled_by: String::new(),
                described_by: String::new(),
                active_descendant: String::new(),
            }),
        );

        tx.set_children(NodeId::ROOT, &[disabled_slider, escaped]);
        tx.commit_into(&mut changes);
        (disabled_slider, escaped)
    };

    // 1. Disabled capture target excluded from WebEventState and SSR `data-nve-capture`.
    let mut ev_state = WebEventState::new();
    ev_state.sync(&store, &changes, |_, _, _| {});
    assert!(!ev_state.has_capture_primary(disabled_slider));
    assert_eq!(ev_state.capture_count, 0);

    let mut web = WebBackend::new();
    web.commit(&store, &changes).unwrap();
    let ssr = web.renderer().render_ssr(&store, NodeId::ROOT);
    assert!(
        !ssr.contains("data-nve-capture=\"primary\""),
        "disabled capture target must not emit data-nve-capture: {ssr}"
    );
    assert!(
        !ssr.contains("role=\"navigation\"") && !ssr.contains("aria-label=\"root-nav\""),
        "NodeId::ROOT Accessibility must stay suppressed in SSR to match live DOM: {ssr}"
    );

    // 2. HTML escaping in Accessibility SSR attributes.
    assert!(
        ssr.contains("aria-label=\"Save &amp; &quot;Close&quot; &lt;now&gt;\""),
        "expected escaped aria-label in SSR: {ssr}"
    );
    assert!(
        ssr.contains("aria-description=\"Press &lt;Enter&gt; &amp; &quot;confirm&quot;\""),
        "expected escaped aria-description in SSR: {ssr}"
    );
    assert!(
        ssr.contains("aria-valuetext=\"1 &lt; 2 &amp; &quot;3&quot;\""),
        "expected escaped aria-valuetext in SSR: {ssr}"
    );
    assert!(
        ssr.contains("aria-controls=\"p&amp;1&quot;&lt;x&gt;\""),
        "expected escaped aria-controls in SSR: {ssr}"
    );

    // 3. Re-enabling slider turns capture on; detaching Accessibility clears all emitted attrs.
    {
        let mut tx = store.tx();
        tx.edit::<Interaction>(disabled_slider).disabled(false);
        tx.detach::<Accessibility>(escaped_node);
        tx.commit_into(&mut changes);
    }
    ev_state.sync(&store, &changes, |_, _, _| {});
    assert!(ev_state.has_capture_primary(disabled_slider));
    assert_eq!(ev_state.capture_count, 1);

    let mut remaining = Vec::new();
    visit_accessibility_attrs(&store, escaped_node, |k, _| remaining.push(k));
    assert!(remaining.is_empty(), "detached Accessibility must emit 0 attrs");

    web.commit(&store, &changes).unwrap();
    let ssr2 = web.renderer().render_ssr(&store, NodeId::ROOT);
    assert!(ssr2.contains("data-nve-capture=\"primary\""));
    assert!(!ssr2.contains("aria-label=\"Save"));
    assert!(!ssr2.contains("aria-controls="));
}
