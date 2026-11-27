//! Unified 3-layer style & layout composition for [`NveMutationWriter`]:
//!
//! 1. `nve_style` ([`TypedStyle`], lowest precedence)
//! 2. Standalone typed attributes (`flex`, `flex-basis`, `padding`, `color`, `border`, ...)
//! 3. Inline `style="..."` override (highest precedence)
//!
//! Composing all three layers through one deterministic pass makes attribute order irrelevant,
//! preserves `flex_grow`/`flex_shrink` alongside `flex-basis` across recomputes, cleanly resets
//! removed attributes without leaking `RawPropValueRef::Null`, and keeps `currentColor` borders
//! symbolic for cross-attribute and ancestor-color resolution.

use std::borrow::Cow;

use nve_abi::{ColorRgba, Dimension, LayoutProps, NodeId, RawPropValueRef, StyleProps};
use nve_components::layout_ext::LayoutExt;
use nve_components::{apply_typed_attr_with_ext, ExtStyleDecl};
use nve_engine::RenderSink;

use crate::typed_style::TypedStyle;

use super::attr_keys::sync_raw_style_prop;
use super::{mark_ext_dirty, NodeRecord, NveMutationWriter, EXT_LAYOUT_EXT};

/// Effective [`LayoutExt`] combining `typed` with an optional `flex-basis` override from
/// standalone attributes or inline `style` (`None` when all fields are default).
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
fn set_node_border_current_color(counter: &mut u32, rec: &mut NodeRecord, v: bool) -> bool {
    let prev = rec.border_uses_current_color();
    if prev == v {
        return false;
    }
    rec.set_border_uses_current_color(v);
    if v {
        *counter = counter.saturating_add(1);
    } else {
        *counter = counter.saturating_sub(1);
    }
    true
}

impl NveMutationWriter {
    /// Resolves the inherited `color` from the nearest ancestor with an explicit `color`
    /// declaration, defaulting to `StyleProps::default().text_color`.
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

    /// Upserts a standalone typed layout/style attribute (under its canonical kebab-case key) and
    /// recomputes the node's effective 3-layer layout, style, and `LayoutExt`.
    pub(super) fn upsert_standalone_typed_attr<S: RenderSink>(
        &mut self,
        sink: &mut S,
        node_id: NodeId,
        key: &'static str,
        value: &str,
    ) {
        let Some(rec) = self.nodes.get_mut(&node_id) else {
            return;
        };
        let ext = rec.extras_mut();
        if let Some((_, existing)) = ext
            .standalone_typed_attrs
            .iter_mut()
            .find(|(k, _)| *k == key)
        {
            if existing == value {
                return;
            }
            existing.clear();
            existing.push_str(value);
        } else {
            ext.standalone_typed_attrs.push((key, value.to_owned()));
        }
        self.recompute_node_styles(sink, node_id);
    }

    /// Removes a standalone typed layout/style attribute and recomputes the node cleanly without
    /// emitting a raw prop `Null`.
    pub(super) fn remove_standalone_typed_attr<S: RenderSink>(
        &mut self,
        sink: &mut S,
        node_id: NodeId,
        key: &'static str,
    ) {
        let Some(rec) = self.nodes.get_mut(&node_id) else {
            return;
        };
        let Some(ext) = rec.extras.as_deref_mut() else {
            return;
        };
        let Some(pos) = ext
            .standalone_typed_attrs
            .iter()
            .position(|(k, _)| *k == key)
        else {
            return;
        };
        ext.standalone_typed_attrs.remove(pos);
        self.recompute_node_styles(sink, node_id);
    }

    /// Recomputes a node's layout, visual style, `LayoutExt::flex_basis`, tracked raw style props,
    /// and symbolic `currentColor` flags in strict layer order:
    /// `nve_style` -> standalone typed attributes -> inline `style`.
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

        let (
            mut next_layout,
            mut next_style,
            mut style_touched,
            mut explicit_text_color,
            mut next_fw,
            mut next_ta,
            mut next_bs,
            mut next_ov,
            mut next_of,
        ) = if let Some(typed) = typed_ref {
            (
                if typed.has_layout {
                    typed.layout
                } else {
                    LayoutProps::default()
                },
                if typed.has_style {
                    typed.style
                } else {
                    StyleProps::default()
                },
                typed.has_style,
                typed.has_text_color,
                typed.font_weight.clone(),
                typed.text_align.clone(),
                typed.box_shadow.clone(),
                typed.overflow.clone(),
                typed.object_fit.clone(),
            )
        } else {
            (
                LayoutProps::default(),
                StyleProps::default(),
                false,
                false,
                None,
                None,
                None,
                None,
                None,
            )
        };

        let mut override_basis: Option<Dimension> = None;
        let mut border_current_color = false;

        if let Some(ext) = ext_ref {
            // Layer 2: standalone typed layout/style attributes.
            for (k, v) in &ext.standalone_typed_attrs {
                let before_style = next_style;
                let _ = apply_typed_attr_with_ext(
                    &mut next_layout,
                    &mut next_style,
                    k,
                    v,
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
                        ExtStyleDecl::Raw(..) => {}
                    },
                );
                if next_style != before_style {
                    style_touched = true;
                }
            }

            // Layer 3: inline `style="..."` override (highest precedence).
            if let Some(override_css) = ext.inline_style_override.as_deref() {
                let before_style = next_style;
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
                        ExtStyleDecl::Raw("font-weight", v) => {
                            next_fw = Some(Cow::Owned(v.to_owned()));
                        }
                        ExtStyleDecl::Raw("text-align", v) => {
                            next_ta = Some(Cow::Owned(v.to_owned()));
                        }
                        ExtStyleDecl::Raw("box-shadow", v) => {
                            next_bs = Some(Cow::Owned(v.to_owned()));
                        }
                        ExtStyleDecl::Raw("overflow", v) => {
                            next_ov = Some(Cow::Owned(v.to_owned()));
                        }
                        ExtStyleDecl::Raw("object-fit", v) => {
                            next_of = Some(Cow::Owned(v.to_owned()));
                        }
                        ExtStyleDecl::Raw(k, v) => {
                            sink.set_raw_prop(node_id, k, RawPropValueRef::String(v));
                        }
                    },
                );
                if next_style != before_style {
                    style_touched = true;
                }
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

        rec.set_has_explicit_style(style_touched);
        rec.set_has_explicit_text_color(explicit_text_color);
        let border_flag_changed = set_node_border_current_color(
            &mut self.current_color_border_nodes,
            rec,
            border_current_color,
        );
        if prev_explicit_text_color != explicit_text_color || border_flag_changed {
            self.dirty_text_props.push(node_id);
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
        let has_any_raw_style = sync_fw
            || sync_ta
            || next_bs.is_some()
            || next_ov.is_some()
            || next_of.is_some()
            || rec.extras.is_some();
        let (fw_changed, ta_changed) = if has_any_raw_style {
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
            let _ = sync_raw_style_prop(sink, node_id, "box-shadow", &mut ext.box_shadow, next_bs);
            let _ = sync_raw_style_prop(sink, node_id, "overflow", &mut ext.overflow, next_ov);
            let _ = sync_raw_style_prop(sink, node_id, "object-fit", &mut ext.object_fit, next_of);
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
