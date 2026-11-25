//! Attribute application, direct zero-string-parse [`TypedStyle`] patching, deterministic
//! `nve_style` + inline `style` override merging, and ARIA key canonicalization.

use std::borrow::Cow;

use dioxus_core::{AttributeValue, ElementId};
use nve_abi::{CommandBufferBuilder, LayoutProps, NodeId, RawPropValue, RawPropValueRef, StyleProps};
use nve_components::{
    apply_typed_attr_with_raw, canonical_raw_style_key, DrawListDecoder, MediaQuery,
    DRAW_LIST_ATTR_PREFIX,
};
use nve_interop::native_view::NATIVE_VIEW_KIND_PROP_KEY;

use crate::hot_patch;
use crate::native_view::{
    NativeBinaryProp, NativeHandleAttr, NATIVE_VIEW_HANDLE_ATTR, NATIVE_VIEW_KIND_ATTR,
};
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
                    .is_some_and(|r| r.deferred_native_create);
                if was_deferred {
                    self.ensure_native_node_created(node_id, Some(kind_str));
                    self.builder
                        .set_raw_prop_str(node_id, NATIVE_VIEW_KIND_PROP_KEY, kind_str);
                } else if let Some(rec) = self.nodes.get_mut(&node_id)
                    && rec.native_kind.as_deref() != Some(kind_str.as_str())
                {
                    rec.native_kind = Some(kind_str.clone());
                    self.builder
                        .set_raw_prop_str(node_id, NATIVE_VIEW_KIND_PROP_KEY, kind_str);
                }
            }
            return;
        }
        if self
            .nodes
            .get(&node_id)
            .is_some_and(|r| r.deferred_native_create)
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
                let prop_name = canonical_a11y_attr_key(name).unwrap_or(name);
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
                    if let Some(rec) = self.nodes.get_mut(&node_id) {
                        rec.explicit_show_on = None;
                        rec.show_on = rec.typed_style.as_ref().and_then(|t| t.show_on);
                    }
                    return;
                }
                if hot_patch::is_test_id_attr_name(name) {
                    self.clear_test_id(node_id);
                }
                let prop_name = canonical_raw_style_key(name)
                    .or_else(|| canonical_a11y_attr_key(name))
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
                        .is_some_and(|rec| rec.typed_style.as_ref() == Some(typed))
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
                        rec.explicit_show_on = Some(query);
                        rec.show_on = Some(query);
                    }
                } else if let Some(bin) = any.as_any().downcast_ref::<NativeBinaryProp>() {
                    self.builder.set_raw_prop_ref(
                        node_id,
                        name,
                        RawPropValueRef::Bytes(&bin.0),
                    );
                } else if name == NATIVE_VIEW_HANDLE_ATTR
                    && let Some(handle_attr) = any.as_any().downcast_ref::<NativeHandleAttr>()
                {
                    let surface_id = self.surface_id;
                    if let Some(rec) = self.nodes.get_mut(&node_id) {
                        if let Some(prev) = rec.native_handle_binder.take()
                            && !handle_attr
                                .0
                                .as_ref()
                                .is_some_and(|next| next.ptr_eq(&prev))
                        {
                            prev.unbind();
                        }
                        if let Some(ref binder) = handle_attr.0 {
                            binder.bind(surface_id, node_id);
                            rec.native_handle_binder = Some(binder.clone());
                        }
                    }
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
        let reset_layout = rec.typed_style.as_ref().is_some_and(|t| t.has_layout)
            || rec.inline_style_override.is_some();
        let reset_style = rec.typed_style.as_ref().is_some_and(|t| t.has_style)
            || rec.inline_style_override.is_some();
        rec.show_on = rec.explicit_show_on.or(typed.show_on);
        rec.responsive_rules.clone_from(&typed.responsive);
        rec.cursor = typed.cursor;
        rec.state_rules.clone_from(&typed.state_rules);
        rec.typed_style = Some(typed);
        self.recompute_typed_style(node_id, reset_layout, reset_style);
    }

    fn clear_typed_style(&mut self, node_id: NodeId) {
        let Some(rec) = self.nodes.get_mut(&node_id) else {
            return;
        };
        let Some(prev) = rec.typed_style.take() else {
            return;
        };
        let reset_layout = prev.has_layout || rec.inline_style_override.is_some();
        let reset_style = prev.has_style || rec.inline_style_override.is_some();
        rec.show_on = rec.explicit_show_on;
        rec.responsive_rules.clear();
        rec.cursor = None;
        rec.state_rules.clear();
        self.recompute_typed_style(node_id, reset_layout, reset_style);
    }

    fn clear_inline_style(&mut self, node_id: NodeId) {
        let Some(rec) = self.nodes.get_mut(&node_id) else {
            return;
        };
        if rec.inline_style_override.take().is_some() && rec.typed_style.is_some() {
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
            if let Some(ref typed) = rec.typed_style {
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
        if let Some(ref override_css) = rec.inline_style_override {
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
            rec.has_explicit_style = true;
            self.dirty_style.push(node_id);
        } else if typed_has_style {
            rec.has_explicit_style = true;
        }

        let builder = &mut self.builder;
        let fw_changed = sync_raw_style_prop(builder, node_id, "font-weight", &mut rec.font_weight, next_fw);
        rec.has_explicit_font_weight = rec.font_weight.is_some();
        let ta_changed = sync_raw_style_prop(builder, node_id, "text-align", &mut rec.text_align, next_ta);
        rec.has_explicit_text_align = rec.text_align.is_some();
        let _ = sync_raw_style_prop(builder, node_id, "box-shadow", &mut rec.box_shadow, next_bs);
        let _ = sync_raw_style_prop(builder, node_id, "overflow", &mut rec.overflow, next_ov);
        let _ = sync_raw_style_prop(builder, node_id, "object-fit", &mut rec.object_fit, next_of);

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
                if rec.typed_style.is_some() {
                    if rec.inline_style_override != next_override {
                        rec.inline_style_override = next_override;
                        self.recompute_typed_style(node_id, true, true);
                    }
                    return;
                }
                rec.inline_style_override = next_override;
            }
        }
        if let Some(rec) = self.nodes.get_mut(&node_id) {
            let prev_layout = rec.layout;
            let prev_style = rec.style;
            let mut text_prop_changed = false;
            let builder = &mut self.builder;
            let handled = apply_typed_attr_with_raw(
                &mut rec.layout,
                &mut rec.style,
                name,
                value,
                |raw_k, raw_v| {
                    match raw_k {
                        "font-weight" => {
                            rec.has_explicit_font_weight = true;
                            if rec.font_weight.as_deref() != Some(raw_v) {
                                rec.font_weight = Some(Cow::Owned(raw_v.to_owned()));
                                text_prop_changed = true;
                            }
                        }
                        "text-align" => {
                            rec.has_explicit_text_align = true;
                            if rec.text_align.as_deref() != Some(raw_v) {
                                rec.text_align = Some(Cow::Owned(raw_v.to_owned()));
                                text_prop_changed = true;
                            }
                        }
                        "box-shadow" => {
                            rec.box_shadow = Some(Cow::Owned(raw_v.to_owned()));
                        }
                        "overflow" => {
                            rec.overflow = Some(Cow::Owned(raw_v.to_owned()));
                        }
                        "object-fit" => {
                            rec.object_fit = Some(Cow::Owned(raw_v.to_owned()));
                        }
                        _ => {}
                    }
                    builder.set_raw_prop_str(node_id, raw_k, raw_v);
                },
            );
            if handled {
                if rec.layout != prev_layout {
                    self.dirty_layout.push(node_id);
                }
                if rec.style != prev_style {
                    rec.has_explicit_style = true;
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
                        rec.has_explicit_font_weight = true;
                        rec.font_weight = Some(Cow::Owned(value.to_owned()));
                        self.dirty_text_props.push(node_id);
                    }
                    "text-align" => {
                        rec.has_explicit_text_align = true;
                        rec.text_align = Some(Cow::Owned(value.to_owned()));
                        self.dirty_text_props.push(node_id);
                    }
                    "box-shadow" => {
                        rec.box_shadow = Some(Cow::Owned(value.to_owned()));
                    }
                    "overflow" => {
                        rec.overflow = Some(Cow::Owned(value.to_owned()));
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
