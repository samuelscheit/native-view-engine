//! Zero-allocation direct fast path + unified deferred 3-layer style & layout composition for
//! [`NveMutationWriter`]:
//!
//! - **Direct fast path** (plain nodes with standalone typed attributes only — no `nve_style`,
//!   inline `style`, or standalone raw-style attrs): writes directly into `NodeRecord.layout` /
//!   `NodeRecord.style` with a compact `u32` `standalone_mask` — **zero `Box<NodeExtras>`, zero
//!   `Vec`, zero `String` allocations, O(k) total parsing**.
//! - **Layered mode** (allocated lazily on first `nve_style`, inline `style`, or standalone
//!   raw-style attribute, promoting any existing direct fields without re-parsing): composes ONCE
//!   per dirty node at `finish_frame` in strict layer order:
//!   1. `nve_style` ([`TypedStyle`], lowest precedence)
//!   2. Standalone typed (`StandaloneLayer` bitmask) + standalone raw-style attributes
//!   3. Inline `style="..."` override (highest precedence)
//! - Tracks every emitted non-inherited raw style prop in `NodeExtras::emitted_raw_styles` so
//!   updates emit zero redundant `set_raw_prop` calls and removals always clear with `Null`.

use std::borrow::Cow;

use nve_abi::{ColorRgba, Dimension, LayoutProps, NodeId, RawPropValueRef, StyleProps};
use nve_components::layout_ext::LayoutExt;
use nve_components::{apply_typed_attr_kind, apply_typed_attr_with_ext, ExtStyleDecl, TypedAttrKind};
use nve_engine::RenderSink;

use crate::typed_style::TypedStyle;

use super::attr_keys::sync_raw_style_prop;
use super::standalone_mask::{
    apply_to_standalone_layer, clear_attr_fields, conflicts_with_direct, mask_for_attr,
    overlay_standalone_layer, promote_direct_to_layer, remove_from_standalone_layer, shorthand_bit,
    MASK_ANY_STYLE,
};
use super::{mark_ext_dirty, NodeRecord, NveMutationWriter, StandaloneLayer, EXT_LAYOUT_EXT};

/// Effective [`LayoutExt`] combining `typed` with an optional `flex-basis` override (`None` when
/// all fields are default).
pub(super) fn effective_layout_ext(
    typed: Option<&TypedStyle>,
    override_basis: Option<Dimension>,
) -> Option<LayoutExt> {
    let typed_ext = typed.and_then(|t| t.layout_ext.as_deref());
    let mut ext = match (typed_ext, override_basis) {
        (None, None) => return None,
        (Some(e), _) => e.clone(),
        (None, Some(_)) => LayoutExt::default(),
    };
    if let Some(basis) = override_basis {
        ext.flex_basis = basis;
    }
    (!ext.is_default()).then_some(ext)
}

#[inline]
fn has_layered_styles(rec: &NodeRecord) -> bool {
    rec.is_dirty_compose()
        || rec.extras().is_some_and(|e| {
            e.typed_style.is_some()
                || e.standalone_layer.is_some()
                || e.inline_style_override.is_some()
                || !e.standalone_raw_attrs.is_empty()
        })
}

#[inline]
pub(super) fn promote_direct_attrs_if_needed(rec: &mut NodeRecord) {
    if rec.standalone_mask == 0 && rec.direct_shorthands == 0 {
        return;
    }
    let mask = core::mem::take(&mut rec.standalone_mask);
    let direct_sh = core::mem::take(&mut rec.direct_shorthands);
    let layout = rec.layout;
    let style = rec.style;
    let border_current_color = rec.border_uses_current_color();
    let flex_basis = rec
        .extras()
        .and_then(|e| e.layout_ext.as_ref())
        .map_or(Dimension::Auto, |le| le.flex_basis);
    let ext = rec.extras_mut();
    if ext.standalone_layer.is_none() {
        ext.standalone_layer = Some(Box::new(promote_direct_to_layer(
            mask,
            direct_sh,
            layout,
            style,
            flex_basis,
            border_current_color,
        )));
    }
}

#[inline]
fn set_node_border_current_color(
    list: &mut Vec<NodeId>,
    rec: &mut NodeRecord,
    node_id: NodeId,
    v: bool,
) -> bool {
    let prev = rec.border_uses_current_color();
    if prev == v {
        return false;
    }
    rec.set_border_uses_current_color(v);
    if v {
        list.push(node_id);
    } else if let Some(pos) = list.iter().position(|&id| id == node_id) {
        list.swap_remove(pos);
    }
    true
}

#[inline]
fn upsert_raw_scratch(
    scratch: &mut Vec<(&'static str, Cow<'static, str>)>,
    key: &'static str,
    val: Cow<'static, str>,
) {
    if let Some((_, existing)) = scratch.iter_mut().find(|(k, _)| *k == key) {
        *existing = val;
    } else {
        scratch.push((key, val));
    }
}

impl NveMutationWriter {
    #[inline]
    pub(super) fn mark_dirty_compose(&mut self, node_id: NodeId) {
        if let Some(rec) = self.nodes.get_mut(&node_id)
            && !rec.is_dirty_compose()
        {
            rec.set_dirty_compose(true);
            self.dirty_compose.push(node_id);
        }
    }

    /// Resolves inherited `color` from the nearest ancestor with an explicit `color` declaration,
    /// defaulting to `StyleProps::default().text_color`.
    pub(super) fn inherited_text_color_from_ancestors(&self, node_id: NodeId) -> ColorRgba {
        let mut cur = self.nodes.get(&node_id).and_then(NodeRecord::parent);
        while let Some(pid) = cur {
            let Some(rec) = self.nodes.get(&pid) else {
                break;
            };
            if rec.has_explicit_text_color() {
                return rec.style.text_color;
            }
            cur = rec.parent();
        }
        StyleProps::default().text_color
    }

    /// Applies one standalone typed attribute: zero-alloc direct path when the node has no
    /// `nve_style`/inline `style`/raw-style layers, else updates `StandaloneLayer` in place and
    /// defers composition until `finish_frame`.
    pub(super) fn apply_standalone_typed_attr(
        &mut self,
        node_id: NodeId,
        kind: TypedAttrKind,
        value: &str,
    ) {
        let Some(rec) = self.nodes.get_mut(&node_id) else {
            return;
        };
        if !has_layered_styles(rec)
            && !conflicts_with_direct(rec.standalone_mask, rec.direct_shorthands, kind)
        {
            let bits = mask_for_attr(kind);
            let is_style_attr = (bits & MASK_ANY_STYLE) != 0;
            let prev_layout = rec.layout;
            let prev_style = rec.style;
            let mut next_style = if is_style_attr && !rec.has_explicit_style() {
                StyleProps::default()
            } else {
                rec.style
            };
            let mut parsed_basis = None;
            let mut border_current = None;
            let mut explicit_color = false;
            let applied = apply_typed_attr_kind(
                &mut rec.layout,
                &mut next_style,
                kind,
                value,
                |decl| match decl {
                    ExtStyleDecl::FlexBasis(b) => parsed_basis = Some(b),
                    ExtStyleDecl::BorderCurrentColor(flag) => border_current = Some(flag),
                    ExtStyleDecl::ExplicitTextColor => explicit_color = true,
                    ExtStyleDecl::ExplicitStyle | ExtStyleDecl::Raw(..) => {}
                },
            );
            if !applied {
                // Invalid CSS value falls back by removing any stale previous value for `kind`.
                self.remove_standalone_typed_attr(node_id, kind);
                return;
            }
            rec.style = next_style;
            rec.standalone_mask |= bits;
            rec.direct_shorthands |= shorthand_bit(kind);
            if let Some(basis) = parsed_basis {
                let next_ext = effective_layout_ext(None, Some(basis));
                if rec.extras().and_then(|e| e.layout_ext.as_ref()) != next_ext.as_ref() {
                    let ext = rec.extras_mut();
                    ext.layout_ext = next_ext;
                    mark_ext_dirty(&mut self.dirty_extended, ext, node_id, EXT_LAYOUT_EXT);
                }
            }
            if let Some(flag) = border_current {
                let _ = set_node_border_current_color(
                    &mut self.current_color_borders,
                    rec,
                    node_id,
                    flag,
                );
                if flag {
                    self.color_tree_dirty = true;
                }
            }
            if explicit_color {
                let prev = rec.has_explicit_text_color();
                rec.set_has_explicit_text_color(true);
                if !prev || rec.style.text_color != prev_style.text_color {
                    self.color_tree_dirty = true;
                }
            }
            if rec.layout != prev_layout {
                self.dirty_layout.push(node_id);
            }
            if is_style_attr {
                rec.set_has_explicit_style(true);
            }
            if rec.style != prev_style {
                self.dirty_style.push(node_id);
            }
            return;
        }

        promote_direct_attrs_if_needed(rec);
        let layer = rec
            .extras_mut()
            .standalone_layer
            .get_or_insert_with(|| Box::new(StandaloneLayer::default()));
        if apply_to_standalone_layer(layer, kind, value) || remove_from_standalone_layer(layer, kind)
        {
            self.mark_dirty_compose(node_id);
        }
    }

    /// Removes one standalone typed attribute cleanly without emitting `RawPropValueRef::Null`.
    pub(super) fn remove_standalone_typed_attr(&mut self, node_id: NodeId, kind: TypedAttrKind) {
        let Some(rec) = self.nodes.get_mut(&node_id) else {
            return;
        };
        let bits = mask_for_attr(kind);
        let sh = shorthand_bit(kind);
        if !has_layered_styles(rec) {
            let was_set_directly = if sh != 0 {
                (rec.direct_shorthands & sh) != 0
            } else {
                (rec.standalone_mask & bits) != 0
                    && !conflicts_with_direct(rec.standalone_mask, rec.direct_shorthands, kind)
            };
            if !was_set_directly {
                return;
            }
            rec.standalone_mask &= !bits;
            rec.direct_shorthands &= !sh;
            let prev_layout = rec.layout;
            let prev_style = rec.style;
            let mut basis = rec
                .extras()
                .and_then(|e| e.layout_ext.as_ref())
                .map_or(Dimension::Auto, |le| le.flex_basis);
            let mut border_current = rec.border_uses_current_color();
            clear_attr_fields(
                kind,
                &mut rec.layout,
                &mut rec.style,
                &mut basis,
                &mut border_current,
            );
            if matches!(kind, TypedAttrKind::Flex | TypedAttrKind::FlexBasis) {
                let next_ext = effective_layout_ext(None, None);
                if rec.extras().and_then(|e| e.layout_ext.as_ref()) != next_ext.as_ref() {
                    let ext = rec.extras_mut();
                    ext.layout_ext = next_ext;
                    mark_ext_dirty(&mut self.dirty_extended, ext, node_id, EXT_LAYOUT_EXT);
                }
            }
            if kind == TypedAttrKind::Color {
                rec.set_has_explicit_text_color(false);
                self.color_tree_dirty = true;
            }
            if matches!(kind, TypedAttrKind::Border | TypedAttrKind::BorderColor) {
                let _ = set_node_border_current_color(
                    &mut self.current_color_borders,
                    rec,
                    node_id,
                    false,
                );
            }
            if (rec.standalone_mask & MASK_ANY_STYLE) == 0 && rec.has_explicit_style() {
                rec.set_has_explicit_style(false);
                if let Some(pid) = rec.parent() {
                    self.dirty_text_props.push(pid);
                }
            }
            if rec.layout != prev_layout {
                self.dirty_layout.push(node_id);
            }
            if rec.style != prev_style {
                self.dirty_style.push(node_id);
            }
            return;
        }

        let Some(layer) = rec.extras.as_deref_mut().and_then(|e| e.standalone_layer.as_deref_mut())
        else {
            return;
        };
        if remove_from_standalone_layer(layer, kind) {
            self.mark_dirty_compose(node_id);
        }
    }

    /// Upserts a standalone raw-style attribute (`"overflow"`, `"font-weight"`, ...) into Layer 2.
    pub(super) fn apply_standalone_raw_attr(
        &mut self,
        node_id: NodeId,
        key: &'static str,
        value: &str,
    ) {
        let Some(rec) = self.nodes.get_mut(&node_id) else {
            return;
        };
        promote_direct_attrs_if_needed(rec);
        let ext = rec.extras_mut();
        if let Some((_, existing)) = ext.standalone_raw_attrs.iter_mut().find(|(k, _)| *k == key) {
            if existing == value {
                return;
            }
            existing.clear();
            existing.push_str(value);
        } else {
            ext.standalone_raw_attrs.push((key, value.to_owned()));
        }
        self.mark_dirty_compose(node_id);
    }

    /// Removes a standalone raw-style attribute from Layer 2 and schedules recomposition.
    pub(super) fn remove_standalone_raw_attr(&mut self, node_id: NodeId, key: &'static str) {
        let Some(rec) = self.nodes.get_mut(&node_id) else {
            return;
        };
        let Some(ext) = rec.extras.as_deref_mut() else {
            return;
        };
        let Some(pos) = ext.standalone_raw_attrs.iter().position(|(k, _)| *k == key) else {
            return;
        };
        ext.standalone_raw_attrs.remove(pos);
        self.mark_dirty_compose(node_id);
    }

    /// Runs deferred 3-layer composition ONCE per dirty node during `finish_frame`.
    pub(super) fn recompute_node_styles<S: RenderSink>(&mut self, sink: &mut S, node_id: NodeId) {
        let Some(rec) = self.nodes.get(&node_id) else {
            return;
        };
        let prev_layout = rec.layout;
        let prev_style = rec.style;
        let had_explicit_style = rec.has_explicit_style();
        let had_explicit_fw = rec.has_explicit_font_weight();
        let had_explicit_ta = rec.has_explicit_text_align();
        let prev_explicit_text_color = rec.has_explicit_text_color();
        let parent_id = rec.parent();

        let ext_ref = rec.extras();
        let typed_ref = ext_ref.and_then(|e| e.typed_style.as_ref());

        self.raw_style_scratch.clear();
        let (
            mut next_layout,
            mut next_style,
            mut style_touched,
            mut explicit_text_color,
            mut next_fw,
            mut next_ta,
        ) = if let Some(typed) = typed_ref {
            if let Some(ref v) = typed.box_shadow {
                upsert_raw_scratch(&mut self.raw_style_scratch, "box-shadow", v.clone());
            }
            if let Some(ref v) = typed.overflow {
                upsert_raw_scratch(&mut self.raw_style_scratch, "overflow", v.clone());
            }
            if let Some(ref v) = typed.object_fit {
                upsert_raw_scratch(&mut self.raw_style_scratch, "object-fit", v.clone());
            }
            (
                if typed.has_layout { typed.layout } else { LayoutProps::default() },
                if typed.has_style { typed.style } else { StyleProps::default() },
                typed.has_style,
                typed.has_text_color,
                typed.font_weight.clone(),
                typed.text_align.clone(),
            )
        } else {
            (LayoutProps::default(), StyleProps::default(), false, false, None, None)
        };

        let mut override_basis: Option<Dimension> = None;
        let mut border_current_color = false;

        if let Some(ext) = ext_ref {
            // Layer 2: standalone typed attributes (zero parsing) + standalone raw-style attrs.
            if let Some(layer) = ext.standalone_layer.as_deref() {
                overlay_standalone_layer(
                    layer,
                    &mut next_layout,
                    &mut next_style,
                    &mut override_basis,
                    &mut border_current_color,
                    &mut explicit_text_color,
                    &mut style_touched,
                );
            }
            for (k, v) in &ext.standalone_raw_attrs {
                match *k {
                    "font-weight" => next_fw = Some(Cow::Owned(v.clone())),
                    "text-align" => next_ta = Some(Cow::Owned(v.clone())),
                    other => upsert_raw_scratch(&mut self.raw_style_scratch, other, Cow::Owned(v.clone())),
                }
            }

            // Layer 3: inline `style="..."` override (parsed once per frame).
            if let Some(override_css) = ext.inline_style_override.as_deref() {
                let _ = apply_typed_attr_with_ext(
                    &mut next_layout,
                    &mut next_style,
                    "style",
                    override_css,
                    |decl| match decl {
                        ExtStyleDecl::FlexBasis(basis) => override_basis = Some(basis),
                        ExtStyleDecl::BorderCurrentColor(flag) => {
                            border_current_color = flag;
                            style_touched = true;
                        }
                        ExtStyleDecl::ExplicitTextColor => {
                            explicit_text_color = true;
                            style_touched = true;
                        }
                        ExtStyleDecl::ExplicitStyle => {
                            style_touched = true;
                        }
                        ExtStyleDecl::Raw("font-weight", v) => {
                            next_fw = Some(Cow::Owned(v.to_owned()));
                        }
                        ExtStyleDecl::Raw("text-align", v) => {
                            next_ta = Some(Cow::Owned(v.to_owned()));
                        }
                        ExtStyleDecl::Raw(k, v) => {
                            upsert_raw_scratch(&mut self.raw_style_scratch, k, Cow::Owned(v.to_owned()));
                        }
                    },
                );
            }
        }

        let next_layout_ext = effective_layout_ext(typed_ref, override_basis);

        if border_current_color {
            next_style.border_color = if explicit_text_color {
                next_style.text_color
            } else {
                self.inherited_text_color_from_ancestors(node_id)
            };
        }
        if !style_touched {
            if !had_explicit_style {
                next_style = prev_style;
            } else if let Some(pid) = parent_id {
                self.dirty_text_props.push(pid);
            }
        }

        let Some(rec) = self.nodes.get_mut(&node_id) else {
            return;
        };
        rec.set_dirty_compose(false);
        rec.set_has_explicit_style(style_touched);
        rec.set_has_explicit_text_color(explicit_text_color);
        let border_flag_changed = set_node_border_current_color(
            &mut self.current_color_borders,
            rec,
            node_id,
            border_current_color,
        );
        if (border_flag_changed && border_current_color)
            || prev_explicit_text_color != explicit_text_color
            || (explicit_text_color && next_style.text_color != prev_style.text_color)
        {
            self.color_tree_dirty = true;
        }

        if next_layout != prev_layout {
            rec.layout = next_layout;
            self.dirty_layout.push(node_id);
        }
        if next_style != prev_style {
            rec.style = next_style;
            self.dirty_style.push(node_id);
        }

        if rec.extras().and_then(|e| e.layout_ext.as_ref()) != next_layout_ext.as_ref() {
            let ext = rec.extras_mut();
            ext.layout_ext = next_layout_ext;
            mark_ext_dirty(&mut self.dirty_extended, ext, node_id, EXT_LAYOUT_EXT);
        }

        let has_fw = next_fw.is_some();
        let has_ta = next_ta.is_some();
        let sync_fw = has_fw || had_explicit_fw;
        let sync_ta = has_ta || had_explicit_ta;
        let need_raw_diff = !self.raw_style_scratch.is_empty()
            || rec.extras().is_some_and(|e| !e.emitted_raw_styles.is_empty());

        let (fw_changed, ta_changed) = if sync_fw || sync_ta || need_raw_diff {
            let ext = rec.extras_mut();
            let fw_c = if sync_fw {
                sync_raw_style_prop(sink, node_id, "font-weight", &mut ext.font_weight, next_fw)
            } else {
                false
            };
            let ta_c = if sync_ta {
                sync_raw_style_prop(sink, node_id, "text-align", &mut ext.text_align, next_ta)
            } else {
                false
            };
            if need_raw_diff {
                // Emit Null for any previously sent raw style key that disappeared.
                ext.emitted_raw_styles.retain(|(k, _)| {
                    if self.raw_style_scratch.iter().any(|(nk, _)| nk == k) {
                        true
                    } else {
                        sink.set_raw_prop(node_id, k, RawPropValueRef::Null);
                        false
                    }
                });
                // Emit String only for keys that are new or whose value changed.
                for (k, v) in self.raw_style_scratch.drain(..) {
                    match ext.emitted_raw_styles.iter_mut().find(|(ek, _)| *ek == k) {
                        Some((_, existing)) => {
                            if existing.as_ref() != v.as_ref() {
                                sink.set_raw_prop(node_id, k, RawPropValueRef::String(&v));
                                *existing = v;
                            }
                        }
                        None => {
                            sink.set_raw_prop(node_id, k, RawPropValueRef::String(&v));
                            ext.emitted_raw_styles.push((k, v));
                        }
                    }
                }
            }
            (fw_c, ta_c)
        } else {
            (false, false)
        };
        rec.set_has_explicit_font_weight(has_fw);
        rec.set_has_explicit_text_align(has_ta);

        if fw_changed || ta_changed {
            self.dirty_text_props.push(node_id);
            if ((!has_fw && had_explicit_fw) || (!has_ta && had_explicit_ta))
                && let Some(pid) = parent_id
            {
                self.dirty_text_props.push(pid);
            }
        }
    }
}
