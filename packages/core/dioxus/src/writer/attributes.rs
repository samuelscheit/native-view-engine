//! Attribute application, direct zero-string-parse [`TypedStyle`] patching, deterministic
//! `nve_style` + inline `style` override merging, extended-prop dirty tracking, and ARIA key
//! canonicalization.

use std::borrow::Cow;

use dioxus_core::{AttributeValue, ElementId};
use nve_abi::{LayoutProps, NodeId, RawPropValueRef, StyleProps};
use nve_components::{
    apply_typed_attr_with_ext, canonical_raw_style_key, AutofillHint, DrawList, DrawListDecoder,
    ExtStyleDecl, InputMode, MediaQuery, SvgPicture, DRAW_LIST_ATTR_PREFIX,
};
use nve_engine::RenderSink;
use nve_interop::native_view::NATIVE_VIEW_KIND_PROP_KEY;

use crate::hot_patch;
use crate::native_view::{
    NativeHandleAttr, NativeTypedPropsAttr, NATIVE_VIEW_HANDLE_ATTR, NATIVE_VIEW_KIND_ATTR,
};
use crate::svg::{SvgPictureAttr, NVE_SVG_PICTURE_ATTR};
use crate::typed_style::{ShowOn, TypedStyle, NVE_SHOW_ON_ATTR, NVE_TYPED_STYLE_ATTR};

use super::attr_keys::{canonical_a11y_attr_key, canonical_input_attr_key, sync_raw_style_prop};
use super::flex_basis::{
    is_flex_basis_attr, set_attr_flex_basis, set_inline_flex_basis, sync_effective_layout_ext,
};
use super::{
    mark_ext_dirty, NveMutationWriter, EXT_BACKGROUND_FILL, EXT_CURSOR, EXT_DRAW_LIST,
    EXT_NATIVE_VIEW_PROPS, EXT_RESPONSIVE_RULES, EXT_SHOW_ON, EXT_STATE_RULES, EXT_SVG,
    EXT_TRANSFORM, EXT_TRANSITIONS,
};

const NVE_SHOW_ON_KEBAB_ATTR: &str = "nve-show-on";

impl NveMutationWriter {
    pub(super) fn set_attribute_impl<S: RenderSink>(
        &mut self,
        sink: &mut S,
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
                    self.ensure_native_node_created(sink, node_id, Some(kind_str));
                    sink.set_raw_prop(
                        node_id,
                        NATIVE_VIEW_KIND_PROP_KEY,
                        RawPropValueRef::String(kind_str),
                    );
                } else if let Some(rec) = self.nodes.get_mut(&node_id)
                    && rec.extras().and_then(|e| e.native_kind.as_deref()) != Some(kind_str.as_str())
                {
                    rec.extras_mut().native_kind = Some(kind_str.clone());
                    sink.set_raw_prop(
                        node_id,
                        NATIVE_VIEW_KIND_PROP_KEY,
                        RawPropValueRef::String(kind_str),
                    );
                }
            }
            return;
        }
        if self
            .nodes
            .get(&node_id)
            .is_some_and(super::NodeRecord::deferred_native_create)
        {
            self.ensure_native_node_created(sink, node_id, None);
        }
        match value {
            AttributeValue::Text(s) => self.apply_str_attr(sink, node_id, name, s),
            AttributeValue::Float(f) => {
                let mut s = String::new();
                nve_abi::num::write_f64(&mut s, *f);
                self.apply_str_attr(sink, node_id, name, &s);
            }
            AttributeValue::Int(i) => {
                let mut s = String::new();
                nve_abi::num::write_i64(&mut s, *i);
                self.apply_str_attr(sink, node_id, name, &s);
            }
            AttributeValue::Bool(b) => {
                let prop_name = canonical_a11y_attr_key(name)
                    .or_else(|| canonical_input_attr_key(name))
                    .unwrap_or(name);
                sink.set_raw_prop(node_id, prop_name, RawPropValueRef::Bool(*b));
            }
            AttributeValue::None => {
                if name == "style" {
                    self.clear_inline_style(sink, node_id);
                    return;
                }
                if name == NVE_TYPED_STYLE_ATTR {
                    self.clear_typed_style(sink, node_id);
                    return;
                }
                if name == NVE_SHOW_ON_KEBAB_ATTR || name == NVE_SHOW_ON_ATTR {
                    if let Some(rec) = self.nodes.get_mut(&node_id)
                        && let Some(ext) = rec.extras.as_deref_mut()
                    {
                        ext.explicit_show_on = None;
                        let next = ext.typed_style.as_ref().and_then(|t| t.show_on);
                        if ext.show_on != next {
                            ext.show_on = next;
                            mark_ext_dirty(&mut self.dirty_extended, ext, node_id, EXT_SHOW_ON);
                        }
                    }
                    return;
                }
                if matches!(name, "drawList" | "draw_list") {
                    if let Some(rec) = self.nodes.get_mut(&node_id)
                        && let Some(ext) = rec.extras.as_deref_mut()
                        && ext.draw_list.take().is_some()
                    {
                        mark_ext_dirty(&mut self.dirty_extended, ext, node_id, EXT_DRAW_LIST);
                    }
                    return;
                }
                if name == NVE_SVG_PICTURE_ATTR || name == "picture" {
                    if let Some(rec) = self.nodes.get_mut(&node_id)
                        && let Some(ext) = rec.extras.as_deref_mut()
                        && ext.svg_picture.take().is_some()
                    {
                        mark_ext_dirty(&mut self.dirty_extended, ext, node_id, EXT_SVG);
                    }
                    return;
                }
                if hot_patch::is_test_id_attr_name(name) {
                    self.clear_test_id(node_id);
                }
                if is_flex_basis_attr(name) {
                    if let Some(rec) = self.nodes.get_mut(&node_id) {
                        set_attr_flex_basis(&mut self.dirty_extended, rec, node_id, None);
                    }
                    return;
                }
                let prop_name = canonical_raw_style_key(name)
                    .or_else(|| canonical_a11y_attr_key(name))
                    .or_else(|| canonical_input_attr_key(name))
                    .unwrap_or(name);
                sink.set_raw_prop(node_id, prop_name, RawPropValueRef::Null);
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
                    self.apply_typed_style(sink, node_id, typed.clone());
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
                        if ext.show_on != Some(query) {
                            ext.show_on = Some(query);
                            mark_ext_dirty(&mut self.dirty_extended, ext, node_id, EXT_SHOW_ON);
                        }
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
                        mark_ext_dirty(&mut self.dirty_extended, ext, node_id, EXT_DRAW_LIST);
                    }
                } else if let Some(typed_nv) = any.as_any().downcast_ref::<NativeTypedPropsAttr>() {
                    if let Some(rec) = self.nodes.get_mut(&node_id) {
                        let ext = rec.extras_mut();
                        if ext.native_props.as_ref() == Some(&typed_nv.0) {
                            return;
                        }
                        ext.native_props = Some(typed_nv.0.clone());
                        mark_ext_dirty(
                            &mut self.dirty_extended,
                            ext,
                            node_id,
                            EXT_NATIVE_VIEW_PROPS,
                        );
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
                        mark_ext_dirty(&mut self.dirty_extended, ext, node_id, EXT_SVG);
                    }
                } else if let Some(hint) = any.as_any().downcast_ref::<AutofillHint>() {
                    sink.set_raw_prop(
                        node_id,
                        "autocomplete",
                        RawPropValueRef::String(hint.as_html_autocomplete()),
                    );
                } else if let Some(mode) = any.as_any().downcast_ref::<InputMode>() {
                    sink.set_raw_prop(
                        node_id,
                        "inputmode",
                        RawPropValueRef::String(mode.as_html_inputmode()),
                    );
                }
            }
            AttributeValue::Listener(_) => {}
        }
    }

    /// Applies a [`TypedStyle`] directly into `NodeRecord` with zero string formatting and zero
    /// CSS parsing when no inline `style` override is present.
    pub(crate) fn apply_typed_style<S: RenderSink>(
        &mut self,
        sink: &mut S,
        node_id: NodeId,
        typed: TypedStyle,
    ) {
        let Some(rec) = self.nodes.get_mut(&node_id) else {
            return;
        };
        let ext = rec.extras_mut();
        let reset_layout = ext.typed_style.as_ref().is_some_and(|t| t.has_layout)
            || ext.inline_style_override.is_some();
        let reset_style = ext.typed_style.as_ref().is_some_and(|t| t.has_style)
            || ext.inline_style_override.is_some();

        let next_show = ext.explicit_show_on.or(typed.show_on);
        if ext.show_on != next_show {
            ext.show_on = next_show;
            mark_ext_dirty(&mut self.dirty_extended, ext, node_id, EXT_SHOW_ON);
        }
        if ext.responsive_rules != typed.responsive {
            ext.responsive_rules.clone_from(&typed.responsive);
            mark_ext_dirty(&mut self.dirty_extended, ext, node_id, EXT_RESPONSIVE_RULES);
        }
        if ext.cursor != typed.cursor {
            ext.cursor = typed.cursor;
            mark_ext_dirty(&mut self.dirty_extended, ext, node_id, EXT_CURSOR);
        }
        if ext.state_rules != typed.state_rules {
            ext.state_rules.clone_from(&typed.state_rules);
            mark_ext_dirty(&mut self.dirty_extended, ext, node_id, EXT_STATE_RULES);
        }
        if ext.transitions != typed.transitions {
            ext.transitions.clone_from(&typed.transitions);
            mark_ext_dirty(&mut self.dirty_extended, ext, node_id, EXT_TRANSITIONS);
        }
        if ext.transform != typed.transform {
            ext.transform = typed.transform;
            mark_ext_dirty(&mut self.dirty_extended, ext, node_id, EXT_TRANSFORM);
        }
        if ext.background_fill != typed.background_fill {
            ext.background_fill = typed.background_fill.clone();
            mark_ext_dirty(&mut self.dirty_extended, ext, node_id, EXT_BACKGROUND_FILL);
        }
        ext.typed_style = Some(typed);
        self.recompute_typed_style(sink, node_id, reset_layout, reset_style);
    }

    fn clear_typed_style<S: RenderSink>(&mut self, sink: &mut S, node_id: NodeId) {
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
        if ext.show_on != ext.explicit_show_on {
            ext.show_on = ext.explicit_show_on;
            mark_ext_dirty(&mut self.dirty_extended, ext, node_id, EXT_SHOW_ON);
        }
        if !ext.responsive_rules.is_empty() {
            ext.responsive_rules.clear();
            mark_ext_dirty(&mut self.dirty_extended, ext, node_id, EXT_RESPONSIVE_RULES);
        }
        if ext.cursor.take().is_some() {
            mark_ext_dirty(&mut self.dirty_extended, ext, node_id, EXT_CURSOR);
        }
        if !ext.state_rules.is_empty() {
            ext.state_rules.clear();
            mark_ext_dirty(&mut self.dirty_extended, ext, node_id, EXT_STATE_RULES);
        }
        if !ext.transitions.is_empty() {
            ext.transitions.clear();
            mark_ext_dirty(&mut self.dirty_extended, ext, node_id, EXT_TRANSITIONS);
        }
        if ext.transform.take().is_some() {
            mark_ext_dirty(&mut self.dirty_extended, ext, node_id, EXT_TRANSFORM);
        }
        if ext.background_fill.take().is_some() {
            mark_ext_dirty(&mut self.dirty_extended, ext, node_id, EXT_BACKGROUND_FILL);
        }
        self.recompute_typed_style(sink, node_id, reset_layout, reset_style);
    }

    fn clear_inline_style<S: RenderSink>(&mut self, sink: &mut S, node_id: NodeId) {
        let Some(rec) = self.nodes.get_mut(&node_id) else {
            return;
        };
        let Some(ext) = rec.extras.as_deref_mut() else {
            return;
        };
        if ext.inline_style_override.take().is_none() {
            return;
        }
        ext.inline_flex_basis = None;
        if ext.typed_style.is_some() {
            self.recompute_typed_style(sink, node_id, true, true);
        } else {
            sync_effective_layout_ext(&mut self.dirty_extended, rec, node_id);
        }
    }

    fn recompute_typed_style<S: RenderSink>(
        &mut self,
        sink: &mut S,
        node_id: NodeId,
        reset_layout: bool,
        reset_style: bool,
    ) {
        let Some(rec) = self.nodes.get_mut(&node_id) else {
            return;
        };
        let prev_layout = rec.layout;
        let prev_style = rec.style;

        let (
            mut next_layout,
            mut next_style,
            typed_has_style,
            mut next_fw,
            mut next_ta,
            mut next_bs,
            mut next_ov,
            mut next_of,
        ) = if let Some(typed) = rec.extras().and_then(|e| e.typed_style.as_ref()) {
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
        let mut inline_basis = None;
        if let Some(override_css) = rec.extras().and_then(|e| e.inline_style_override.as_deref()) {
            let _ = apply_typed_attr_with_ext(
                &mut next_layout,
                &mut next_style,
                "style",
                override_css,
                |decl| match decl {
                    ExtStyleDecl::FlexBasis(basis) => inline_basis = Some(basis),
                    ExtStyleDecl::Raw("font-weight", v) => next_fw = Some(Cow::Owned(v.to_owned())),
                    ExtStyleDecl::Raw("text-align", v) => next_ta = Some(Cow::Owned(v.to_owned())),
                    ExtStyleDecl::Raw("box-shadow", v) => next_bs = Some(Cow::Owned(v.to_owned())),
                    ExtStyleDecl::Raw("overflow", v) => next_ov = Some(Cow::Owned(v.to_owned())),
                    ExtStyleDecl::Raw("object-fit", v) => next_of = Some(Cow::Owned(v.to_owned())),
                    ExtStyleDecl::Raw(k, v) => extra_raw.push((k.to_owned(), v.to_owned())),
                },
            );
        }
        set_inline_flex_basis(&mut self.dirty_extended, rec, node_id, inline_basis);

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

        let has_any_raw_style = next_fw.is_some()
            || next_ta.is_some()
            || next_bs.is_some()
            || next_ov.is_some()
            || next_of.is_some()
            || rec.extras.is_some();
        let (fw_changed, ta_changed, has_fw, has_ta) = if has_any_raw_style {
            let ext = rec.extras_mut();
            let fw_c = sync_raw_style_prop(sink, node_id, "font-weight", &mut ext.font_weight, next_fw);
            let ta_c = sync_raw_style_prop(sink, node_id, "text-align", &mut ext.text_align, next_ta);
            let _ = sync_raw_style_prop(sink, node_id, "box-shadow", &mut ext.box_shadow, next_bs);
            let _ = sync_raw_style_prop(sink, node_id, "overflow", &mut ext.overflow, next_ov);
            let _ = sync_raw_style_prop(sink, node_id, "object-fit", &mut ext.object_fit, next_of);
            (fw_c, ta_c, ext.font_weight.is_some(), ext.text_align.is_some())
        } else {
            (false, false, false, false)
        };
        rec.set_has_explicit_font_weight(has_fw);
        rec.set_has_explicit_text_align(has_ta);

        for (k, v) in extra_raw {
            sink.set_raw_prop(node_id, &k, RawPropValueRef::String(&v));
        }

        if fw_changed || ta_changed {
            self.dirty_text_props.push(node_id);
        }
    }

    pub(crate) fn apply_str_attr<S: RenderSink>(
        &mut self,
        sink: &mut S,
        node_id: NodeId,
        name: &str,
        value: &str,
    ) {
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
                        self.recompute_typed_style(sink, node_id, true, true);
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
            let extras_slot = &mut rec.extras;
            let mut parsed_basis = None;
            let handled = apply_typed_attr_with_ext(
                &mut rec.layout,
                &mut rec.style,
                name,
                value,
                |decl| {
                    let (raw_k, raw_v) = match decl {
                        ExtStyleDecl::FlexBasis(basis) => {
                            parsed_basis = Some(basis);
                            return;
                        }
                        ExtStyleDecl::Raw(k, v) => (k, v),
                    };
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
                    sink.set_raw_prop(node_id, raw_k, RawPropValueRef::String(raw_v));
                },
            );
            if set_fw_flag {
                rec.set_has_explicit_font_weight(true);
            }
            if set_ta_flag {
                rec.set_has_explicit_text_align(true);
            }
            if name == "style" {
                set_inline_flex_basis(&mut self.dirty_extended, rec, node_id, parsed_basis);
            } else if let Some(basis) = parsed_basis {
                set_attr_flex_basis(&mut self.dirty_extended, rec, node_id, Some(basis));
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
        // Decode legacy `nve-draw1;b64,...` string attributes directly into binary `RawPropValueRef::Bytes`.
        if value.starts_with(DRAW_LIST_ATTR_PREFIX)
            && let Ok(raw_draw_bytes) = DrawListDecoder::decode_attr_bytes(value)
        {
            sink.set_raw_prop(node_id, name, RawPropValueRef::Bytes(&raw_draw_bytes));
            return;
        }
        let prop_name = canonical_raw_style_key(name)
            .or_else(|| canonical_a11y_attr_key(name))
            .or_else(|| canonical_input_attr_key(name))
            .unwrap_or(name);
        sink.set_raw_prop(node_id, prop_name, RawPropValueRef::String(value));
    }
}
