//! Attribute application, direct zero-string-parse [`TypedStyle`] patching, deterministic
//! `nve_style` + inline `style` override merging, and ARIA key canonicalization.

use std::borrow::Cow;

use dioxus_core::{AttributeValue, ElementId};
use nve_abi::{CommandBufferBuilder, LayoutProps, NodeId, RawPropValue, RawPropValueRef, StyleProps};
use nve_components::{
    apply_typed_attr_with_raw, canonical_raw_style_key, AutofillHint, DrawList, DrawListDecoder,
    InputMode, MediaQuery, SvgPicture, DRAW_LIST_ATTR_PREFIX,
};
use nve_interop::native_view::NATIVE_VIEW_KIND_PROP_KEY;

use crate::hot_patch;
use crate::native_view::{
    NativeHandleAttr, NativeTypedPropsAttr, NATIVE_VIEW_HANDLE_ATTR, NATIVE_VIEW_KIND_ATTR,
};
use crate::svg::{SvgPictureAttr, NVE_SVG_PICTURE_ATTR};
use crate::typed_style::{ShowOn, TypedStyle, NVE_SHOW_ON_ATTR, NVE_TYPED_STYLE_ATTR};

const NVE_SHOW_ON_KEBAB_ATTR: &str = "nve-show-on";

use super::NveMutationWriter;

impl NveMutationWriter {
    pub(super) fn set_attribute_impl(
        &mut self,
        name: &'static str,
        value: &AttributeValue,
        id: ElementId,
    ) {
        let Some(node_id) = self.resolve_el(id) else {
            return;
        };
        if name == NATIVE_VIEW_KIND_ATTR {
            if let AttributeValue::Text(kind_str) = value {
                let was_deferred = self
                    .nodes
                    .get(&node_id)
                    .is_some_and(super::NodeRecord::deferred_native_create);
                if was_deferred {
                    self.ensure_native_node_created(node_id, Some(kind_str));
                    self.builder
                        .set_raw_prop_str(node_id, NATIVE_VIEW_KIND_PROP_KEY, kind_str);
                } else if let Some(rec) = self.nodes.get_mut(&node_id)
                    && rec.extras().and_then(|e| e.native_kind.as_deref()) != Some(kind_str.as_str())
                {
                    rec.extras_mut().native_kind = Some(kind_str.clone());
                    self.builder
                        .set_raw_prop_str(node_id, NATIVE_VIEW_KIND_PROP_KEY, kind_str);
                }
            }
            return;
        }
        if self
            .nodes
            .get(&node_id)
            .is_some_and(super::NodeRecord::deferred_native_create)
        {
            self.ensure_native_node_created(node_id, None);
        }
        match value {
            AttributeValue::Text(s) => self.apply_str_attr(node_id, name, s),
            AttributeValue::Float(f) => {
                let mut s = String::new();
                nve_abi::num::write_f64(&mut s, *f);
                self.apply_str_attr(node_id, name, &s);
            }
            AttributeValue::Int(i) => {
                let mut s = String::new();
                nve_abi::num::write_i64(&mut s, *i);
                self.apply_str_attr(node_id, name, &s);
            }
            AttributeValue::Bool(b) => {
                let prop_name = canonical_a11y_attr_key(name)
                    .or_else(|| canonical_input_attr_key(name))
                    .unwrap_or(name);
                self.builder
                    .set_raw_prop(node_id, prop_name, &RawPropValue::Bool(*b));
            }
            AttributeValue::None => {
                if name == "style" {
                    self.clear_inline_style(node_id);
                    return;
                }
                if name == NVE_TYPED_STYLE_ATTR {
                    self.clear_typed_style(node_id);
                    return;
                }
                if name == NVE_SHOW_ON_KEBAB_ATTR || name == NVE_SHOW_ON_ATTR {
                    if let Some(rec) = self.nodes.get_mut(&node_id)
                        && let Some(ext) = rec.extras.as_deref_mut()
                    {
                        ext.explicit_show_on = None;
                        ext.show_on = ext.typed_style.as_ref().and_then(|t| t.show_on);
                    }
                    return;
                }
                if matches!(name, "drawList" | "draw_list") {
                    if let Some(rec) = self.nodes.get_mut(&node_id)
                        && let Some(ext) = rec.extras.as_deref_mut()
                    {
                        ext.draw_list = None;
                    }
                    if self.in_process_mode {
                        return;
                    }
                }
                if name == NVE_SVG_PICTURE_ATTR || name == "picture" {
                    if let Some(rec) = self.nodes.get_mut(&node_id)
                        && let Some(ext) = rec.extras.as_deref_mut()
                    {
                        ext.svg_picture = None;
                    }
                    return;
                }
                if hot_patch::is_test_id_attr_name(name) {
                    self.clear_test_id(node_id);
                }
                let prop_name = canonical_raw_style_key(name)
                    .or_else(|| canonical_a11y_attr_key(name))
                    .or_else(|| canonical_input_attr_key(name))
                    .unwrap_or(name);
                self.builder
                    .set_raw_prop(node_id, prop_name, &RawPropValue::Null);
            }
            AttributeValue::Any(any) => {
                if name == NVE_TYPED_STYLE_ATTR
                    && let Some(typed) = any.as_any().downcast_ref::<TypedStyle>()
                {
                    if self
                        .nodes
                        .get(&node_id)
                        .and_then(|rec| rec.extras())
                        .is_some_and(|ext| ext.typed_style.as_ref() == Some(typed))
                    {
                        return;
                    }
                    self.apply_typed_style(node_id, typed.clone());
                } else if (name == NVE_SHOW_ON_KEBAB_ATTR || name == NVE_SHOW_ON_ATTR)
                    && let Some(query) = any
                        .as_any()
                        .downcast_ref::<MediaQuery>()
                        .copied()
                        .or_else(|| any.as_any().downcast_ref::<ShowOn>().map(|s| s.0))
                {
                    if let Some(rec) = self.nodes.get_mut(&node_id) {
                        let ext = rec.extras_mut();
                        ext.explicit_show_on = Some(query);
                        ext.show_on = Some(query);
                    }
                } else if let Some(draw_list) = any.as_any().downcast_ref::<DrawList>() {
                    if let Some(rec) = self.nodes.get_mut(&node_id) {
                        let ext = rec.extras_mut();
                        if ext
                            .draw_list
                            .as_ref()
                            .is_some_and(|prev| prev.ptr_eq(draw_list) || prev == draw_list)
                        {
                            return;
                        }
                        ext.draw_list = Some(draw_list.clone());
                    }
                    if !self.in_process_mode {
                        let bytes = draw_list.to_nve_draw1_bytes();
                        self.builder
                            .set_raw_prop_ref(node_id, "drawList", RawPropValueRef::Bytes(&bytes));
                    }
                } else if let Some(typed_nv) = any.as_any().downcast_ref::<NativeTypedPropsAttr>() {
                    if let Some(rec) = self.nodes.get_mut(&node_id) {
                        let ext = rec.extras_mut();
                        if ext.native_props.as_ref() == Some(&typed_nv.0) {
                            return;
                        }
                        ext.native_props = Some(typed_nv.0.clone());
                    }
                    if !self.in_process_mode
                        && let Ok(bytes) = typed_nv.0.to_nvnp_bytes()
                    {
                        self.builder
                            .set_raw_prop_ref(node_id, name, RawPropValueRef::Bytes(&bytes));
                    }
                } else if name == NATIVE_VIEW_HANDLE_ATTR
                    && let Some(handle_attr) = any.as_any().downcast_ref::<NativeHandleAttr>()
                {
                    let surface_id = self.surface_id;
                    if let Some(rec) = self.nodes.get_mut(&node_id) {
                        let ext = rec.extras_mut();
                        if let Some(prev) = ext.native_handle_binder.take()
                            && !handle_attr
                                .0
                                .as_ref()
                                .is_some_and(|next| next.ptr_eq(&prev))
                        {
                            prev.unbind();
                        }
                        if let Some(ref binder) = handle_attr.0 {
                            binder.bind(surface_id, node_id);
                            ext.native_handle_binder = Some(binder.clone());
                        }
                    }
                } else if let Some(svg) = any
                    .as_any()
                    .downcast_ref::<SvgPicture>()
                    .or_else(|| any.as_any().downcast_ref::<SvgPictureAttr>().map(|a| &a.0))
                {
                    if let Some(rec) = self.nodes.get_mut(&node_id) {
                        let ext = rec.extras_mut();
                        if ext
                            .svg_picture
                            .as_ref()
                            .is_some_and(|prev| prev.ptr_eq(svg) || prev == svg)
                        {
                            return;
                        }
                        ext.svg_picture = Some(svg.clone());
                    }
                } else if let Some(hint) = any.as_any().downcast_ref::<AutofillHint>() {
                    self.builder
                        .set_raw_prop_str(node_id, "autocomplete", hint.as_html_autocomplete());
                } else if let Some(mode) = any.as_any().downcast_ref::<InputMode>() {
                    self.builder
                        .set_raw_prop_str(node_id, "inputmode", mode.as_html_inputmode());
                }
            }
            AttributeValue::Listener(_) => {}
        }
    }

    /// Applies a [`TypedStyle`] directly into `NodeRecord` with zero string formatting and zero
    /// CSS parsing when no inline `style` override is present.
    pub(crate) fn apply_typed_style(&mut self, node_id: NodeId, typed: TypedStyle) {
        let Some(rec) = self.nodes.get_mut(&node_id) else {
            return;
        };
        let ext = rec.extras_mut();
        let reset_layout = ext.typed_style.as_ref().is_some_and(|t| t.has_layout)
            || ext.inline_style_override.is_some();
        let reset_style = ext.typed_style.as_ref().is_some_and(|t| t.has_style)
            || ext.inline_style_override.is_some();
        ext.show_on = ext.explicit_show_on.or(typed.show_on);
        ext.responsive_rules.clone_from(&typed.responsive);
        ext.cursor = typed.cursor;
        ext.state_rules.clone_from(&typed.state_rules);
        ext.transitions.clone_from(&typed.transitions);
        ext.layout_ext = typed
            .layout_ext
            .as_deref()
            .filter(|e| !e.is_default())
            .cloned();
        ext.transform = typed.transform;
        ext.background_fill = typed.background_fill.clone();
        ext.typed_style = Some(typed);
        self.recompute_typed_style(node_id, reset_layout, reset_style);
    }

    fn clear_typed_style(&mut self, node_id: NodeId) {
        let Some(rec) = self.nodes.get_mut(&node_id) else {
            return;
        };
        let Some(ext) = rec.extras.as_deref_mut() else {
            return;
        };
        let Some(prev) = ext.typed_style.take() else {
            return;
        };
        let reset_layout = prev.has_layout || ext.inline_style_override.is_some();
        let reset_style = prev.has_style || ext.inline_style_override.is_some();
        ext.show_on = ext.explicit_show_on;
        ext.responsive_rules.clear();
        ext.cursor = None;
        ext.state_rules.clear();
        ext.transitions.clear();
        ext.layout_ext = None;
        ext.transform = None;
        ext.background_fill = None;
        self.recompute_typed_style(node_id, reset_layout, reset_style);
    }

    fn clear_inline_style(&mut self, node_id: NodeId) {
        let Some(rec) = self.nodes.get_mut(&node_id) else {
            return;
        };
        let should_recompute = rec
            .extras
            .as_deref_mut()
            .is_some_and(|ext| ext.inline_style_override.take().is_some() && ext.typed_style.is_some());
        if should_recompute {
            self.recompute_typed_style(node_id, true, true);
        }
    }

    fn recompute_typed_style(&mut self, node_id: NodeId, reset_layout: bool, reset_style: bool) {
        let Some(rec) = self.nodes.get_mut(&node_id) else {
            return;
        };
        let prev_layout = rec.layout;
        let prev_style = rec.style;

        let (mut next_layout, mut next_style, typed_has_style, mut next_fw, mut next_ta, mut next_bs, mut next_ov, mut next_of) =
            if let Some(typed) = rec.extras().and_then(|e| e.typed_style.as_ref()) {
                (
                    if typed.has_layout {
                        typed.layout
                    } else if reset_layout {
                        LayoutProps::default()
                    } else {
                        prev_layout
                    },
                    if typed.has_style {
                        typed.style
                    } else if reset_style {
                        StyleProps::default()
                    } else {
                        prev_style
                    },
                    typed.has_style,
                    typed.font_weight.clone(),
                    typed.text_align.clone(),
                    typed.box_shadow.clone(),
                    typed.overflow.clone(),
                    typed.object_fit.clone(),
                )
            } else {
                (
                    if reset_layout {
                        LayoutProps::default()
                    } else {
                        prev_layout
                    },
                    if reset_style {
                        StyleProps::default()
                    } else {
                        prev_style
                    },
                    false,
                    None,
                    None,
                    None,
                    None,
                    None,
                )
            };

        let mut extra_raw: Vec<(String, String)> = Vec::new();
        if let Some(override_css) = rec.extras().and_then(|e| e.inline_style_override.as_deref()) {
            let _ = apply_typed_attr_with_raw(
                &mut next_layout,
                &mut next_style,
                "style",
                override_css,
                |raw_k, raw_v| match raw_k {
                    "font-weight" => next_fw = Some(Cow::Owned(raw_v.to_owned())),
                    "text-align" => next_ta = Some(Cow::Owned(raw_v.to_owned())),
                    "box-shadow" => next_bs = Some(Cow::Owned(raw_v.to_owned())),
                    "overflow" => next_ov = Some(Cow::Owned(raw_v.to_owned())),
                    "object-fit" => next_of = Some(Cow::Owned(raw_v.to_owned())),
                    other => extra_raw.push((other.to_owned(), raw_v.to_owned())),
                },
            );
        }

        if next_layout != prev_layout {
            rec.layout = next_layout;
            self.dirty_layout.push(node_id);
        }
        if next_style != prev_style {
            rec.style = next_style;
            rec.set_has_explicit_style(true);
            self.dirty_style.push(node_id);
        } else if typed_has_style {
            rec.set_has_explicit_style(true);
        }

        let builder = &mut self.builder;
        let has_any_raw_style = next_fw.is_some()
            || next_ta.is_some()
            || next_bs.is_some()
            || next_ov.is_some()
            || next_of.is_some()
            || rec.extras.is_some();
        let (fw_changed, ta_changed, has_fw, has_ta) = if has_any_raw_style {
            let ext = rec.extras_mut();
            let fw_c = sync_raw_style_prop(builder, node_id, "font-weight", &mut ext.font_weight, next_fw);
            let ta_c = sync_raw_style_prop(builder, node_id, "text-align", &mut ext.text_align, next_ta);
            let _ = sync_raw_style_prop(builder, node_id, "box-shadow", &mut ext.box_shadow, next_bs);
            let _ = sync_raw_style_prop(builder, node_id, "overflow", &mut ext.overflow, next_ov);
            let _ = sync_raw_style_prop(builder, node_id, "object-fit", &mut ext.object_fit, next_of);
            (fw_c, ta_c, ext.font_weight.is_some(), ext.text_align.is_some())
        } else {
            (false, false, false, false)
        };
        rec.set_has_explicit_font_weight(has_fw);
        rec.set_has_explicit_text_align(has_ta);

        for (k, v) in extra_raw {
            builder.set_raw_prop_str(node_id, &k, &v);
        }

        if fw_changed || ta_changed {
            self.dirty_text_props.push(node_id);
        }
    }

    pub(crate) fn apply_str_attr(&mut self, node_id: NodeId, name: &str, value: &str) {
        if hot_patch::is_test_id_attr_name(name) {
            self.record_test_id(node_id, value);
        }
        if name == "style" {
            let trimmed = value.trim();
            if let Some(rec) = self.nodes.get_mut(&node_id) {
                let next_override = (!trimmed.is_empty()).then(|| trimmed.to_owned());
                if rec.extras().is_some_and(|e| e.typed_style.is_some()) {
                    let ext = rec.extras_mut();
                    if ext.inline_style_override != next_override {
                        ext.inline_style_override = next_override;
                        self.recompute_typed_style(node_id, true, true);
                    }
                    return;
                }
                if next_override.is_some() || rec.extras.is_some() {
                    rec.extras_mut().inline_style_override = next_override;
                }
            }
        }
        if let Some(rec) = self.nodes.get_mut(&node_id) {
            let prev_layout = rec.layout;
            let prev_style = rec.style;
            let mut text_prop_changed = false;
            let mut set_fw_flag = false;
            let mut set_ta_flag = false;
            let builder = &mut self.builder;
            let extras_slot = &mut rec.extras;
            let handled = apply_typed_attr_with_raw(
                &mut rec.layout,
                &mut rec.style,
                name,
                value,
                |raw_k, raw_v| {
                    match raw_k {
                        "font-weight" => {
                            set_fw_flag = true;
                            let e = extras_slot.get_or_insert_with(Box::default);
                            if e.font_weight.as_deref() != Some(raw_v) {
                                e.font_weight = Some(Cow::Owned(raw_v.to_owned()));
                                text_prop_changed = true;
                            }
                        }
                        "text-align" => {
                            set_ta_flag = true;
                            let e = extras_slot.get_or_insert_with(Box::default);
                            if e.text_align.as_deref() != Some(raw_v) {
                                e.text_align = Some(Cow::Owned(raw_v.to_owned()));
                                text_prop_changed = true;
                            }
                        }
                        "box-shadow" => {
                            extras_slot.get_or_insert_with(Box::default).box_shadow =
                                Some(Cow::Owned(raw_v.to_owned()));
                        }
                        "overflow" => {
                            extras_slot.get_or_insert_with(Box::default).overflow =
                                Some(Cow::Owned(raw_v.to_owned()));
                        }
                        "object-fit" => {
                            extras_slot.get_or_insert_with(Box::default).object_fit =
                                Some(Cow::Owned(raw_v.to_owned()));
                        }
                        _ => {}
                    }
                    builder.set_raw_prop_str(node_id, raw_k, raw_v);
                },
            );
            if set_fw_flag {
                rec.set_has_explicit_font_weight(true);
            }
            if set_ta_flag {
                rec.set_has_explicit_text_align(true);
            }
            if handled {
                if rec.layout != prev_layout {
                    self.dirty_layout.push(node_id);
                }
                if rec.style != prev_style {
                    rec.set_has_explicit_style(true);
                    self.dirty_style.push(node_id);
                }
                if text_prop_changed {
                    self.dirty_text_props.push(node_id);
                }
                return;
            }
            if let Some(canonical) = canonical_raw_style_key(name) {
                match canonical {
                    "font-weight" => {
                        rec.set_has_explicit_font_weight(true);
                        rec.extras_mut().font_weight = Some(Cow::Owned(value.to_owned()));
                        self.dirty_text_props.push(node_id);
                    }
                    "text-align" => {
                        rec.set_has_explicit_text_align(true);
                        rec.extras_mut().text_align = Some(Cow::Owned(value.to_owned()));
                        self.dirty_text_props.push(node_id);
                    }
                    "box-shadow" => {
                        rec.extras_mut().box_shadow = Some(Cow::Owned(value.to_owned()));
                    }
                    "overflow" => {
                        rec.extras_mut().overflow = Some(Cow::Owned(value.to_owned()));
                    }
                    _ => {}
                }
            }
        }
        // Decode `nve-draw1;b64,...` attributes directly into binary `RawPropValue::Bytes`
        // in one pass — eliminating any full-frame CommandBuffer decode+re-encode pass.
        if value.starts_with(DRAW_LIST_ATTR_PREFIX)
            && let Ok(raw_draw_bytes) = DrawListDecoder::decode_attr_bytes(value)
        {
            self.builder
                .set_raw_prop(node_id, name, &RawPropValue::Bytes(raw_draw_bytes));
            return;
        }
        let prop_name = canonical_raw_style_key(name)
            .or_else(|| canonical_a11y_attr_key(name))
            .or_else(|| canonical_input_attr_key(name))
            .unwrap_or(name);
        self.builder.set_raw_prop_str(node_id, prop_name, value);
    }
}

#[inline]
fn sync_raw_style_prop(
    builder: &mut CommandBufferBuilder,
    node_id: NodeId,
    key: &str,
    slot: &mut Option<Cow<'static, str>>,
    next: Option<Cow<'static, str>>,
) -> bool {
    if slot.as_deref() == next.as_deref() {
        return false;
    }
    match &next {
        Some(v) => builder.set_raw_prop_str(node_id, key, v),
        None => builder.set_raw_prop(node_id, key, &RawPropValue::Null),
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
