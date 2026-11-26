//! Canonical ARIA & HTML5 input/semantic attribute key normalization for `NveMutationWriter`.

use std::borrow::Cow;

use nve_abi::{NodeId, RawPropValueRef};
use nve_engine::RenderSink;

#[inline]
pub(super) fn sync_raw_style_prop<S: RenderSink>(
    sink: &mut S,
    node_id: NodeId,
    key: &str,
    slot: &mut Option<Cow<'static, str>>,
    next: Option<Cow<'static, str>>,
) -> bool {
    if slot.as_deref() == next.as_deref() {
        return false;
    }
    match &next {
        Some(v) => sink.set_raw_prop(node_id, key, RawPropValueRef::String(v)),
        None => sink.set_raw_prop(node_id, key, RawPropValueRef::Null),
    }
    *slot = next;
    true
}

/// Canonicalizes ARIA state and focus attribute keys (kebab-case, snake_case, or camelCase)
/// into their standard camelCase raw-prop wire names with zero binary ABI changes.
#[inline]
pub(crate) fn canonical_a11y_attr_key(name: &str) -> Option<&'static str> {
    match name {
        "tabIndex" | "tab-index" | "tab_index" => Some("tabIndex"),
        "ariaChecked" | "aria-checked" | "aria_checked" => Some("ariaChecked"),
        "ariaSelected" | "aria-selected" | "aria_selected" => Some("ariaSelected"),
        "ariaExpanded" | "aria-expanded" | "aria_expanded" => Some("ariaExpanded"),
        "ariaModal" | "aria-modal" | "aria_modal" => Some("ariaModal"),
        "ariaLive" | "aria-live" | "aria_live" => Some("ariaLive"),
        "ariaControls" | "aria-controls" | "aria_controls" => Some("ariaControls"),
        "ariaLabelledBy" | "aria-labelledby" | "aria_labelledby" => Some("ariaLabelledBy"),
        "ariaDescribedBy" | "aria-describedby" | "aria_describedby" => Some("ariaDescribedBy"),
        "ariaValueNow" | "aria-valuenow" | "aria_valuenow" => Some("ariaValueNow"),
        "ariaValueMin" | "aria-valuemin" | "aria_valuemin" => Some("ariaValueMin"),
        "ariaValueMax" | "aria-valuemax" | "aria_valuemax" => Some("ariaValueMax"),
        "ariaActiveDescendant" | "aria-activedescendant" | "aria_activedescendant" => {
            Some("ariaActiveDescendant")
        }
        "ariaAutoComplete" | "aria-autocomplete" | "aria_autocomplete" => Some("ariaAutoComplete"),
        "ariaMultiSelectable" | "aria-multiselectable" | "aria_multiselectable" => {
            Some("ariaMultiSelectable")
        }
        "ariaOrientation" | "aria-orientation" | "aria_orientation" => Some("ariaOrientation"),
        "ariaInvalid" | "aria-invalid" | "aria_invalid" => Some("ariaInvalid"),
        "ariaDisabled" | "aria-disabled" | "aria_disabled" => Some("ariaDisabled"),
        _ => None,
    }
}

/// Canonicalizes cross-platform text-input & native HTML5 semantic attribute keys into their
/// canonical HTML/wire names (`autocomplete`, `inputmode`, `closedby`, `commandfor`,
/// `popovertarget`, `popovertargetaction`, `novalidate`, `type`) with zero binary ABI changes.
#[inline]
pub(crate) fn canonical_input_attr_key(name: &str) -> Option<&'static str> {
    match name {
        "autocomplete" | "autoComplete" | "auto_complete" => Some("autocomplete"),
        "inputmode" | "inputMode" | "input_mode" => Some("inputmode"),
        "closedby" | "closedBy" | "closed_by" => Some("closedby"),
        "commandfor" | "commandFor" | "command_for" => Some("commandfor"),
        "popovertarget" | "popoverTarget" | "popover_target" => Some("popovertarget"),
        "popovertargetaction" | "popoverTargetAction" | "popover_target_action" => {
            Some("popovertargetaction")
        }
        "novalidate" | "noValidate" | "no_validate" => Some("novalidate"),
        "button_type" | "input_type" => Some("type"),
        _ => None,
    }
}
