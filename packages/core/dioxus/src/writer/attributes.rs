//! Attribute application, direct zero-string-parse [`TypedStyle`] patching, deterministic
//! `nve_style` + inline `style` override merging, extended-prop dirty tracking, and ARIA key
//! canonicalization.

use dioxus_core::{AttributeValue, ElementId};
use nve_abi::{NodeId, RawPropValueRef};
use nve_components::{
    canonical_raw_style_key, AutofillHint, DrawList, DrawListDecoder, InputMode, MediaQuery,
    SvgPicture, TypedAttrKind, DRAW_LIST_ATTR_PREFIX,
};
use nve_engine::RenderSink;
use nve_interop::native_view::NATIVE_VIEW_KIND_PROP_KEY;

use crate::hot_patch;
use crate::native_view::{
    NativeHandleAttr, NativeTypedPropsAttr, NATIVE_VIEW_HANDLE_ATTR, NATIVE_VIEW_KIND_ATTR,
};
use crate::svg::{SvgPictureAttr, NVE_SVG_PICTURE_ATTR};
use crate::typed_style::{ShowOn, TypedStyle, NVE_SHOW_ON_ATTR, NVE_TYPED_STYLE_ATTR};

use super::attr_keys::{canonical_a11y_attr_key, canonical_input_attr_key};
use super::style_compose::promote_direct_attrs_if_needed;
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
                let mut buf = [0u8; 24];
                let s = nve_abi::num::format_f64(&mut buf, *f);
                self.apply_str_attr(sink, node_id, name, s);
            }
            AttributeValue::Int(i) => {
                let mut buf = [0u8; 24];
                let s = nve_abi::num::format_i64(&mut buf, *i);
                self.apply_str_attr(sink, node_id, name, s);
            }
            AttributeValue::Bool(b) => {
                let prop_name = canonical_a11y_attr_key(name)
                    .or_else(|| canonical_input_attr_key(name))
                    .unwrap_or(name);
                sink.set_raw_prop(node_id, prop_name, RawPropValueRef::Bool(*b));
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
                if let Some(kind) = TypedAttrKind::from_key(name) {
                    self.remove_standalone_typed_attr(node_id, kind);
                    return;
                }
                if let Some(canon) = canonical_raw_style_key(name) {
                    self.remove_standalone_raw_attr(node_id, canon);
                    return;
                }
                let prop_name = canonical_a11y_attr_key(name)
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
    /// CSS parsing when no inline `style` or standalone typed attribute override is present.
    pub(crate) fn apply_typed_style(&mut self, node_id: NodeId, typed: TypedStyle) {
        let Some(rec) = self.nodes.get_mut(&node_id) else {
            return;
        };
        promote_direct_attrs_if_needed(rec);
        let ext = rec.extras_mut();

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
        self.mark_dirty_compose(node_id);
    }

    fn clear_typed_style(&mut self, node_id: NodeId) {
        let Some(rec) = self.nodes.get_mut(&node_id) else {
            return;
        };
        let Some(ext) = rec.extras.as_deref_mut() else {
            return;
        };
        if ext.typed_style.take().is_none() {
            return;
        }
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
        self.mark_dirty_compose(node_id);
    }

    fn clear_inline_style(&mut self, node_id: NodeId) {
        let Some(rec) = self.nodes.get_mut(&node_id) else {
            return;
        };
        let Some(ext) = rec.extras.as_deref_mut() else {
            return;
        };
        if ext.inline_style_override.take().is_none() {
            return;
        }
        self.mark_dirty_compose(node_id);
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
            let next_override = (!trimmed.is_empty()).then_some(trimmed);
            if let Some(rec) = self.nodes.get_mut(&node_id) {
                if rec
                    .extras()
                    .and_then(|e| e.inline_style_override.as_deref())
                    == next_override
                {
                    return;
                }
                promote_direct_attrs_if_needed(rec);
                if let Some(css) = next_override {
                    let ext = rec.extras_mut();
                    match &mut ext.inline_style_override {
                        Some(existing) => {
                            existing.clear();
                            existing.push_str(css);
                        }
                        None => ext.inline_style_override = Some(css.to_owned()),
                    }
                } else if let Some(ext) = rec.extras.as_deref_mut() {
                    ext.inline_style_override = None;
                }
                self.mark_dirty_compose(node_id);
            }
            return;
        }
        if let Some(kind) = TypedAttrKind::from_key(name) {
            self.apply_standalone_typed_attr(node_id, kind, value);
            return;
        }
        if let Some(canon) = canonical_raw_style_key(name) {
            self.apply_standalone_raw_attr(node_id, canon, value);
            return;
        }
        // Decode legacy `nve-draw1;b64,...` string attributes directly into binary `RawPropValueRef::Bytes`.
        if value.starts_with(DRAW_LIST_ATTR_PREFIX)
            && let Ok(raw_draw_bytes) = DrawListDecoder::decode_attr_bytes(value)
        {
            sink.set_raw_prop(node_id, name, RawPropValueRef::Bytes(&raw_draw_bytes));
            return;
        }
        let prop_name = canonical_a11y_attr_key(name)
            .or_else(|| canonical_input_attr_key(name))
            .unwrap_or(name);
        sink.set_raw_prop(node_id, prop_name, RawPropValueRef::String(value));
    }
}
