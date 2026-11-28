//! Single canonical 21-field WAI-ARIA emitter shared identically by SSR
//! ([`crate::table_system::write_core_ssr_attrs`]) and live `wasm32` DOM (`wasm_dom.rs`) so
//! server-rendered markup and hydrated/live DOM attributes can never drift.

use nve_abi::num::{write_f32, write_u64};
use nve_store::{NodeId, NodeStore};
use nve_tables::tables::{
    Accessibility, AriaAutoComplete, AriaOrientation, AriaRelations, AriaTriState,
};

#[inline]
const fn aria_tristate_str(v: AriaTriState) -> &'static str {
    match v {
        AriaTriState::False => "false",
        AriaTriState::True => "true",
        AriaTriState::Mixed => "mixed",
    }
}

#[inline]
const fn aria_orientation_str(v: AriaOrientation) -> &'static str {
    match v {
        AriaOrientation::Horizontal => "horizontal",
        AriaOrientation::Vertical => "vertical",
    }
}

#[inline]
const fn aria_autocomplete_str(v: AriaAutoComplete) -> &'static str {
    match v {
        AriaAutoComplete::None => "none",
        AriaAutoComplete::Inline => "inline",
        AriaAutoComplete::List => "list",
    }
}

/// Exhaustive ordered list of every HTML attribute governed by the 21 fields of [`Accessibility`].
pub const ALL_ACCESSIBILITY_ATTRS: [&str; 24] = [
    "role",
    "aria-level",
    "aria-modal",
    "aria-live",
    "aria-orientation",
    "aria-autocomplete",
    "aria-checked",
    "aria-selected",
    "aria-expanded",
    "aria-pressed",
    "aria-hidden",
    "aria-invalid",
    "aria-multiselectable",
    "aria-busy",
    "aria-label",
    "aria-description",
    "aria-valuetext",
    "aria-valuenow",
    "aria-valuemin",
    "aria-valuemax",
    "aria-controls",
    "aria-labelledby",
    "aria-describedby",
    "aria-activedescendant",
];

/// Visits every present WAI-ARIA attribute for `id`'s [`Accessibility`] row (all 21 fields,
/// including all 4 [`AriaRelations`] ID-ref attributes) in canonical order.
pub fn visit_accessibility_attrs(
    store: &NodeStore,
    id: NodeId,
    mut emit: impl FnMut(&'static str, &str),
) {
    let Some(a11y) = store.try_row::<Accessibility>(id) else {
        return;
    };
    let mut num_buf = String::new();

    // 1. role
    if let Some(role) = a11y.role() {
        emit("role", role.as_aria_str());
    }
    // 2. level
    if let Some(level) = a11y.level() {
        num_buf.clear();
        write_u64(&mut num_buf, u64::from(level));
        emit("aria-level", &num_buf);
    }
    // 3. modal
    if a11y.modal() {
        emit("aria-modal", "true");
    }
    // 4. live
    if let Some(live) = a11y.live() {
        emit("aria-live", live.as_aria_str());
    }
    // 5. orientation
    if let Some(orient) = a11y.orientation() {
        emit("aria-orientation", aria_orientation_str(orient));
    }
    // 6. autocomplete
    if let Some(ac) = a11y.autocomplete() {
        emit("aria-autocomplete", aria_autocomplete_str(ac));
    }
    // 7. checked
    if let Some(checked) = a11y.checked() {
        emit("aria-checked", aria_tristate_str(checked));
    }
    // 8. selected
    if let Some(selected) = a11y.selected() {
        emit("aria-selected", aria_tristate_str(selected));
    }
    // 9. expanded
    if let Some(expanded) = a11y.expanded() {
        emit("aria-expanded", aria_tristate_str(expanded));
    }
    // 10. pressed
    if let Some(pressed) = a11y.pressed() {
        emit("aria-pressed", aria_tristate_str(pressed));
    }
    // 11. hidden
    if a11y.hidden() {
        emit("aria-hidden", "true");
    }
    // 12. invalid
    if a11y.invalid() {
        emit("aria-invalid", "true");
    }
    // 13. multiselectable
    if a11y.multiselectable() {
        emit("aria-multiselectable", "true");
    }
    // 14. busy
    if a11y.busy() {
        emit("aria-busy", "true");
    }
    // 15. label
    let label = a11y.label();
    if !label.is_empty() {
        emit("aria-label", label);
    }
    // 16. hint
    let hint = a11y.hint();
    if !hint.is_empty() {
        emit("aria-description", hint);
    }
    // 17. value_text
    let value_text = a11y.value_text();
    if !value_text.is_empty() {
        emit("aria-valuetext", value_text);
    }
    // 18. value_now
    if let Some(v) = a11y.value_now().filter(|v| v.is_finite()) {
        num_buf.clear();
        write_f32(&mut num_buf, if v == 0.0 { 0.0 } else { v });
        emit("aria-valuenow", &num_buf);
    }
    // 19. value_min
    if let Some(v) = a11y.value_min().filter(|v| v.is_finite()) {
        num_buf.clear();
        write_f32(&mut num_buf, if v == 0.0 { 0.0 } else { v });
        emit("aria-valuemin", &num_buf);
    }
    // 20. value_max
    if let Some(v) = a11y.value_max().filter(|v| v.is_finite()) {
        num_buf.clear();
        write_f32(&mut num_buf, if v == 0.0 { 0.0 } else { v });
        emit("aria-valuemax", &num_buf);
    }
    // 21. relations
    let rel_res = a11y.relations();
    if !rel_res.is_none()
        && let Some(rel) = store.resource::<AriaRelations>(rel_res)
    {
        for (attr, val) in [
            ("aria-controls", rel.controls()),
            ("aria-labelledby", rel.labelled_by()),
            ("aria-describedby", rel.described_by()),
            ("aria-activedescendant", rel.active_descendant()),
        ] {
            if !val.is_empty() {
                emit(attr, val);
            }
        }
    }
}

/// Synchronizes all 24 WAI-ARIA attributes on a live `wasm32` DOM element, setting every active
/// attribute via [`visit_accessibility_attrs`] and removing any stale attributes.
#[cfg(target_arch = "wasm32")]
pub(crate) fn sync_accessibility_dom_attrs(
    store: &NodeStore,
    id: NodeId,
    el: &web_sys::Element,
) {
    let mut present_mask: u32 = 0;
    visit_accessibility_attrs(store, id, |name, val| {
        if let Some(idx) = ALL_ACCESSIBILITY_ATTRS.iter().position(|&a| a == name) {
            present_mask |= 1u32 << idx;
        }
        if el.get_attribute(name).as_deref() != Some(val) {
            let _ = el.set_attribute(name, val);
        }
    });
    for (idx, &attr) in ALL_ACCESSIBILITY_ATTRS.iter().enumerate() {
        if (present_mask & (1u32 << idx)) == 0 && el.has_attribute(attr) {
            let _ = el.remove_attribute(attr);
        }
    }
}
