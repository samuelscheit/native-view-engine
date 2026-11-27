//! Direct zero-string-parse attribute application onto `NodeStore` word tables.
//!
//! - Matches `AttributeValue::Float`/`Int`/`Bool` directly without string formatting/reparsing.
//! - Matches `TypedStyle`, `MediaQuery`, and extensible [`TableAttr`] directly via `AttributeValue::Any`.
//! - Routes inline `style` and CSS shorthands straight into `nve_components::apply_css_attr_to_tx`.
//! - Zero `set_raw_prop`, zero `ComponentKind`, zero module-specific downcasts.

use dioxus_core::{AttributeValue, ElementId};
use nve_components::input_hints::{AutofillHint, InputMode};
use nve_components::media::MediaQuery;
use nve_components::tables::{
    Accessibility, AriaAutoComplete, AriaLive, AriaOrientation, AriaRelations, AriaRole,
    AriaTriState, DebugId, Image, ImageFit, ImageSrc, InputKind, Interaction, Layout, Link,
    LinkRel, LinkTarget, Responsive, TextInput, Typography, Visual,
};
use nve_store::{Length, NodeId};

use crate::table_attr::TableAttr;
use crate::typed_style::{pack_media_query, TypedStyle, NVE_SHOW_ON_ATTR, NVE_TYPED_STYLE_ATTR};

use super::{StandaloneStyleVal, StoreMutationTx};

const NVE_SHOW_ON_KEBAB_ATTR: &str = "nve-show-on";

impl StoreMutationTx<'_, '_> {
    pub(super) fn set_attribute_impl(
        &mut self,
        name: &'static str,
        value: &AttributeValue,
        id: ElementId,
    ) {
        let Some(node) = self.resolve_el(id) else {
            return;
        };

        match value {
            #[expect(
                clippy::disallowed_methods,
                reason = "Dioxus AttributeValue::Any boundary unpacks typed RSX attributes"
            )]
            AttributeValue::Any(any) => {
                if name == NVE_TYPED_STYLE_ATTR
                    && let Some(ts) = any.as_any().downcast_ref::<TypedStyle>()
                {
                    if let Some(prev) = self.typed_styles.get(&node) {
                        prev.clear_diff_from(Some(ts), self.tx, node);
                    }
                    ts.apply_to_tx(self.tx, node);
                    self.typed_styles.insert(node, ts.clone());
                    self.replay_standalone_and_inline(node, nve_components::STYLE_LAYER_ALL);
                    return;
                }
                if (name == NVE_SHOW_ON_ATTR || name == NVE_SHOW_ON_KEBAB_ATTR)
                    && let Some(&q) = any.as_any().downcast_ref::<MediaQuery>()
                {
                    let (w, env) = pack_media_query(q);
                    self.tx
                        .edit::<Responsive>(node)
                        .show_on_widths(w)
                        .show_on_env(env);
                    return;
                }
                if let Some(table_attr) = any.as_any().downcast_ref::<TableAttr>() {
                    let list = self.attached_tables.entry(node).or_default();
                    if let Some((_, prev)) = list.iter_mut().find(|(k, _)| *k == name) {
                        if !prev.same_writer_kind(table_attr) {
                            prev.clear_from(self.tx, node);
                        }
                        *prev = table_attr.clone();
                    } else {
                        list.push((name, table_attr.clone()));
                    }
                    table_attr.write_to(self.tx, node);
                    return;
                }
                // Fallback: allow TypedStyle or MediaQuery even if passed on custom attribute name.
                if let Some(ts) = any.as_any().downcast_ref::<TypedStyle>() {
                    ts.apply_to_tx(self.tx, node);
                } else if let Some(&q) = any.as_any().downcast_ref::<MediaQuery>() {
                    let (w, env) = pack_media_query(q);
                    self.tx
                        .edit::<Responsive>(node)
                        .show_on_widths(w)
                        .show_on_env(env);
                }
            }
            AttributeValue::Float(f) => self.apply_numeric_attr(node, name, *f),
            AttributeValue::Int(i) => self.apply_int_attr(node, name, *i),
            AttributeValue::Bool(b) => self.apply_bool_attr(node, name, *b),
            AttributeValue::Text(s) => self.apply_str_attr(node, name, s),
            AttributeValue::None => self.clear_attr(node, name),
            AttributeValue::Listener(_) => {}
        }
    }

    #[inline]
    fn apply_numeric_attr(&mut self, node: NodeId, name: &str, v: f64) {
        if self.apply_numeric_attr_untracked(node, name, v) {
            self.finish_standalone_style(node, name, StandaloneStyleVal::Float(v));
        }
    }

    /// Applies a numeric attribute directly to the store. Returns `true` iff `name` is a style
    /// attribute that should be preserved across `style`/`nve_style` clears/updates.
    #[inline]
    pub(crate) fn apply_numeric_attr_untracked(&mut self, node: NodeId, name: &str, v: f64) -> bool {
        let f = if v.is_finite() { v as f32 } else { 0.0 };
        match name {
            "width" => {
                self.tx.edit::<Layout>(node).width(Length::points(f.max(0.0)));
            }
            "height" => {
                self.tx.edit::<Layout>(node).height(Length::points(f.max(0.0)));
            }
            "min-width" | "minWidth" => {
                self.tx
                    .edit::<Layout>(node)
                    .min_width(Length::points(f.max(0.0)));
            }
            "min-height" | "minHeight" => {
                self.tx
                    .edit::<Layout>(node)
                    .min_height(Length::points(f.max(0.0)));
            }
            "max-width" | "maxWidth" => {
                self.tx
                    .edit::<Layout>(node)
                    .max_width(Length::points(f.max(0.0)));
            }
            "max-height" | "maxHeight" => {
                self.tx
                    .edit::<Layout>(node)
                    .max_height(Length::points(f.max(0.0)));
            }
            "flex" | "flex-grow" | "flexGrow" => {
                self.tx.edit::<Layout>(node).flex_grow(f.max(0.0));
            }
            "flex-shrink" | "flexShrink" => {
                self.tx.edit::<Layout>(node).flex_shrink(f.max(0.0));
            }
            "gap" => {
                let l = Length::points(f.max(0.0));
                self.tx.edit::<Layout>(node).row_gap(l).column_gap(l);
            }
            "row-gap" | "rowGap" => {
                self.tx.edit::<Layout>(node).row_gap(Length::points(f.max(0.0)));
            }
            "column-gap" | "columnGap" => {
                self.tx
                    .edit::<Layout>(node)
                    .column_gap(Length::points(f.max(0.0)));
            }
            "padding" => {
                let l = Length::points(f.max(0.0));
                self.tx
                    .edit::<Layout>(node)
                    .padding_top(l)
                    .padding_right(l)
                    .padding_bottom(l)
                    .padding_left(l);
            }
            "padding-horizontal" | "paddingHorizontal" => {
                let l = Length::points(f.max(0.0));
                self.tx.edit::<Layout>(node).padding_left(l).padding_right(l);
            }
            "padding-vertical" | "paddingVertical" => {
                let l = Length::points(f.max(0.0));
                self.tx.edit::<Layout>(node).padding_top(l).padding_bottom(l);
            }
            "margin" => {
                let l = Length::points(f);
                self.tx
                    .edit::<Layout>(node)
                    .margin_top(l)
                    .margin_right(l)
                    .margin_bottom(l)
                    .margin_left(l);
            }
            "border-width" | "borderWidth" => {
                self.tx.edit::<Visual>(node).border_width(f.max(0.0));
            }
            "border-radius" | "borderRadius" => {
                self.tx.edit::<Visual>(node).border_radius(f.max(0.0));
            }
            "opacity" => {
                self.tx.edit::<Visual>(node).opacity(f.clamp(0.0, 1.0));
            }
            "font-size" | "fontSize" => {
                let cur_set = self
                    .tx
                    .store()
                    .try_row::<Typography>(node)
                    .map_or(0, |t| t.set())
                    | Typography::SET_FONT_SIZE;
                self.tx
                    .edit::<Typography>(node)
                    .font_size(f.max(1.0))
                    .set(cur_set);
            }
            "z-index" | "zIndex" => {
                self.tx.edit::<Visual>(node).z_index(f as i32);
            }
            "ariaValueNow" | "aria-valuenow" | "aria_valuenow" => {
                self.tx.edit::<Accessibility>(node).value_now(Some(f));
                return false;
            }
            "ariaValueMin" | "aria-valuemin" | "aria_valuemin" => {
                self.tx.edit::<Accessibility>(node).value_min(Some(f));
                return false;
            }
            "ariaValueMax" | "aria-valuemax" | "aria_valuemax" => {
                self.tx.edit::<Accessibility>(node).value_max(Some(f));
                return false;
            }
            _ => return false,
        }
        true
    }

    #[inline]
    fn apply_int_attr(&mut self, node: NodeId, name: &str, v: i64) {
        match name {
            "tabIndex" | "tab-index" | "tab_index" => {
                self.tx
                    .edit::<Interaction>(node)
                    .tab_index(Some(v.clamp(i16::MIN as i64, i16::MAX as i64) as i16));
            }
            "z-index" | "zIndex" => {
                self.tx
                    .edit::<Visual>(node)
                    .z_index(v.clamp(i32::MIN as i64, i32::MAX as i64) as i32);
                self.finish_standalone_style(node, name, StandaloneStyleVal::Int(v));
            }
            "minlength" => {
                let len = (v >= 0).then(|| v.min(i64::from(u16::MAX)) as u16);
                self.tx.edit::<TextInput>(node).min_length(len);
            }
            "maxlength" => {
                let len = (v >= 0).then(|| v.min(i64::from(u16::MAX)) as u16);
                self.tx.edit::<TextInput>(node).max_length(len);
            }
            _ => self.apply_numeric_attr(node, name, v as f64),
        }
    }

    #[inline]
    fn apply_bool_attr(&mut self, node: NodeId, name: &str, b: bool) {
        match name {
            "disabled" | "ariaDisabled" | "aria-disabled" | "aria_disabled" => {
                self.tx.edit::<Interaction>(node).disabled(b);
            }
            "ariaChecked" | "aria-checked" | "aria_checked" => {
                self.tx.edit::<Accessibility>(node).checked(Some(if b {
                    AriaTriState::True
                } else {
                    AriaTriState::False
                }));
            }
            "ariaSelected" | "aria-selected" | "aria_selected" => {
                self.tx.edit::<Accessibility>(node).selected(Some(if b {
                    AriaTriState::True
                } else {
                    AriaTriState::False
                }));
            }
            "ariaExpanded" | "aria-expanded" | "aria_expanded" => {
                self.tx.edit::<Accessibility>(node).expanded(Some(if b {
                    AriaTriState::True
                } else {
                    AriaTriState::False
                }));
            }
            "ariaModal" | "aria-modal" | "aria_modal" => {
                self.tx.edit::<Accessibility>(node).modal(b);
            }
            "ariaInvalid" | "aria-invalid" | "aria_invalid" => {
                self.tx.edit::<Accessibility>(node).invalid(b);
            }
            "ariaMultiSelectable" | "aria-multiselectable" | "aria_multiselectable" => {
                self.tx.edit::<Accessibility>(node).multiselectable(b);
            }
            "multiline" => {
                self.tx.edit::<TextInput>(node).multiline(b);
            }
            "secureTextEntry" | "secure_text_entry" => {
                self.tx.edit::<TextInput>(node).secure(b);
            }
            "readOnly" | "read_only" | "readonly" => {
                self.tx.edit::<TextInput>(node).read_only(b);
            }
            "required" => {
                self.tx.edit::<TextInput>(node).required(b);
            }
            _ => {}
        }
    }

    pub(crate) fn apply_str_attr(&mut self, node: NodeId, name: &str, value: &str) {
        if name == "style" {
            self.apply_inline_style_attr(node, value);
            return;
        }
        match nve_components::apply_css_attr_layers_status_to_tx(
            self.tx,
            node,
            name,
            value,
            nve_components::STYLE_LAYER_ALL,
        ) {
            nve_components::CssAttrResult::Applied(_) => {
                self.finish_standalone_style(node, name, StandaloneStyleVal::Str(value.to_owned()));
                return;
            }
            nve_components::CssAttrResult::Invalid(layer) => {
                debug_assert!(layer != 0, "invalid CSS attr `{name}` must carry a non-zero layer");
                if self.remove_standalone_style(node, name) {
                    self.rebuild_style_layers_mask(node, layer);
                }
                return;
            }
            nve_components::CssAttrResult::Unknown => {}
        }
        match name {
            "testID" | "data-testid" | "test_id" => {
                self.tx.edit::<DebugId>(node).test_id(value);
            }
            "id" => {
                self.tx.edit::<DebugId>(node).id(value);
            }
            "slot" => {
                self.tx.edit::<DebugId>(node).slot(value);
            }
            "src" => {
                self.tx
                    .edit::<Image>(node)
                    .src(Some(ImageSrc::Url(value.to_owned())));
            }
            "alt" => {
                self.tx.edit::<Image>(node).alt(value);
            }
            "resizeMode" | "resize_mode" | "object-fit" | "objectFit" => {
                let fit = ImageFit::parse(value).unwrap_or(ImageFit::Cover);
                self.tx.edit::<Image>(node).fit(fit);
            }
            "href" => {
                self.tx.edit::<Link>(node).href(value);
            }
            "target" => {
                let target = match value.trim() {
                    "_blank" => LinkTarget::Blank,
                    "_self" => LinkTarget::Self_,
                    "_parent" => LinkTarget::Parent,
                    _ => LinkTarget::Default,
                };
                self.tx.edit::<Link>(node).target(target);
            }
            "rel" => {
                let mut rel = LinkRel::EMPTY;
                for tok in value.split_ascii_whitespace() {
                    match tok {
                        "noopener" => rel |= LinkRel::NOOPENER,
                        "noreferrer" => rel |= LinkRel::NOREFERRER,
                        "nofollow" => rel |= LinkRel::NOFOLLOW,
                        _ => {}
                    }
                }
                self.tx.edit::<Link>(node).rel(rel);
            }
            "value" => {
                self.tx.edit::<TextInput>(node).value(value);
            }
            "placeholder" => {
                self.tx.edit::<TextInput>(node).placeholder(value);
            }
            "type" | "input_type" => {
                let kind = InputKind::parse(value).unwrap_or(InputKind::Text);
                self.tx.edit::<TextInput>(node).input_kind(kind);
                if kind == InputKind::Password {
                    self.tx.edit::<TextInput>(node).secure(true);
                }
            }
            "autocomplete" | "autoComplete" | "auto_complete" => {
                let hint = AutofillHint::parse_html_autocomplete(value);
                self.tx.edit::<TextInput>(node).autofill(hint);
            }
            "inputmode" | "inputMode" | "input_mode" => {
                let mode = InputMode::parse_html_inputmode(value).unwrap_or(InputMode::Text);
                self.tx.edit::<TextInput>(node).input_mode(mode);
            }
            "pattern" => {
                self.tx.edit::<TextInput>(node).pattern(value);
            }
            "multiline" | "secureTextEntry" | "secure_text_entry" | "readOnly" | "read_only"
            | "readonly" | "required" | "disabled" => {
                let b = matches!(value.trim(), "" | "true" | "1");
                self.apply_bool_attr(node, name, b);
            }
            "minlength" | "maxlength" | "tabIndex" | "tab-index" | "tab_index" => {
                if let Ok(i) = value.trim().parse::<i64>() {
                    self.apply_int_attr(node, name, i);
                }
            }
            "accessibilityLabel" | "aria-label" | "ariaLabel" => {
                self.tx.edit::<Accessibility>(node).label(value);
            }
            "accessibilityHint" => {
                self.tx.edit::<Accessibility>(node).hint(value);
            }
            "accessibilityRole" | "role" => {
                let role = AriaRole::parse(value);
                self.tx.edit::<Accessibility>(node).role(role);
            }
            "aria-level" | "ariaLevel" | "aria_level" => {
                if let Ok(lvl) = value.trim().parse::<u8>() {
                    self.tx
                        .edit::<Accessibility>(node)
                        .level((lvl > 0).then_some(lvl));
                }
            }
            "ariaChecked" | "aria-checked" | "aria_checked" => {
                self.tx
                    .edit::<Accessibility>(node)
                    .checked(parse_tri_state(value));
            }
            "ariaSelected" | "aria-selected" | "aria_selected" => {
                self.tx
                    .edit::<Accessibility>(node)
                    .selected(parse_tri_state(value));
            }
            "ariaExpanded" | "aria-expanded" | "aria_expanded" => {
                self.tx
                    .edit::<Accessibility>(node)
                    .expanded(parse_tri_state(value));
            }
            "ariaLive" | "aria-live" | "aria_live" => {
                let live = match value.trim() {
                    "polite" => Some(AriaLive::Polite),
                    "assertive" => Some(AriaLive::Assertive),
                    "off" => Some(AriaLive::Off),
                    _ => None,
                };
                self.tx.edit::<Accessibility>(node).live(live);
            }
            "ariaOrientation" | "aria-orientation" | "aria_orientation" => {
                let orient = match value.trim() {
                    "horizontal" => Some(AriaOrientation::Horizontal),
                    "vertical" => Some(AriaOrientation::Vertical),
                    _ => None,
                };
                self.tx.edit::<Accessibility>(node).orientation(orient);
            }
            "ariaAutoComplete" | "aria-autocomplete" | "aria_autocomplete" => {
                let ac = match value.trim() {
                    "inline" => Some(AriaAutoComplete::Inline),
                    "list" | "both" => Some(AriaAutoComplete::List),
                    "none" => Some(AriaAutoComplete::None),
                    _ => None,
                };
                self.tx.edit::<Accessibility>(node).autocomplete(ac);
            }
            "ariaControls" | "aria-controls" | "aria_controls" | "ariaLabelledBy"
            | "aria-labelledby" | "aria_labelledby" | "ariaDescribedBy" | "aria-describedby"
            | "aria_describedby" | "ariaActiveDescendant" | "aria-activedescendant"
            | "aria_activedescendant" => {
                let mut rel = self
                    .tx
                    .store()
                    .try_row::<Accessibility>(node)
                    .and_then(|a| {
                        let r = a.relations();
                        (!r.is_none())
                            .then(|| self.tx.store().resource::<AriaRelations>(r).cloned())
                            .flatten()
                    })
                    .unwrap_or_default();
                match name {
                    "ariaControls" | "aria-controls" | "aria_controls" => {
                        rel.controls = value.to_owned();
                    }
                    "ariaLabelledBy" | "aria-labelledby" | "aria_labelledby" => {
                        rel.labelled_by = value.to_owned();
                    }
                    "ariaDescribedBy" | "aria-describedby" | "aria_describedby" => {
                        rel.described_by = value.to_owned();
                    }
                    _ => {
                        rel.active_descendant = value.to_owned();
                    }
                }
                self.tx.edit::<Accessibility>(node).relations(Some(rel));
            }
            "ariaModal" | "aria-modal" | "aria_modal" | "ariaInvalid" | "aria-invalid"
            | "aria_invalid" | "ariaDisabled" | "aria-disabled" | "aria_disabled"
            | "ariaMultiSelectable" | "aria-multiselectable" | "aria_multiselectable" => {
                let b = matches!(value.trim(), "true" | "1");
                self.apply_bool_attr(node, name, b);
            }
            "ariaValueNow" | "aria-valuenow" | "aria_valuenow" | "ariaValueMin"
            | "aria-valuemin" | "aria_valuemin" | "ariaValueMax" | "aria-valuemax"
            | "aria_valuemax" => {
                if let Ok(v) = value.trim().parse::<f64>() {
                    self.apply_numeric_attr(node, name, v);
                }
            }
            _ => {}
        }
    }

    fn clear_attr(&mut self, node: NodeId, name: &str) {
        if self.remove_standalone_style(node, name) {
            let layer = nve_components::css_attr_layer(name);
            debug_assert!(layer != 0, "removed standalone style `{name}` must have a non-zero layer");
            self.rebuild_style_layers_mask(node, layer);
            return;
        }
        if let Some(list) = self.attached_tables.get_mut(&node)
            && let Some(pos) = list.iter().position(|(k, _)| *k == name)
        {
            let (_, prev) = list.swap_remove(pos);
            prev.clear_from(self.tx, node);
            if list.is_empty() {
                self.attached_tables.remove(&node);
            }
            return;
        }
        match name {
            "style" => {
                if let Some((_, prev_mask)) = self.inline_styles.remove(&node) {
                    let layers = nve_components::prop_bits_to_layers(prev_mask);
                    if layers != 0 {
                        self.rebuild_style_layers_mask(node, layers);
                    }
                }
            }
            NVE_TYPED_STYLE_ATTR => {
                if let Some(prev) = self.typed_styles.remove(&node) {
                    prev.clear_diff_from(None, self.tx, node);
                }
                self.replay_standalone_and_inline(node, nve_components::STYLE_LAYER_ALL);
            }
            NVE_SHOW_ON_ATTR | NVE_SHOW_ON_KEBAB_ATTR => {
                if self.tx.store().has::<Responsive>(node) {
                    self.tx
                        .edit::<Responsive>(node)
                        .show_on_widths(0)
                        .show_on_env(0);
                }
            }
            "testID" | "data-testid" | "test_id" if self.tx.store().has::<DebugId>(node) => {
                self.tx.edit::<DebugId>(node).test_id("");
            }
            "id" if self.tx.store().has::<DebugId>(node) => {
                self.tx.edit::<DebugId>(node).id("");
            }
            "slot" if self.tx.store().has::<DebugId>(node) => {
                self.tx.edit::<DebugId>(node).slot("");
            }
            "href" if self.tx.store().has::<Link>(node) => {
                self.tx.edit::<Link>(node).href("");
            }
            "target" if self.tx.store().has::<Link>(node) => {
                self.tx.edit::<Link>(node).target(LinkTarget::Default);
            }
            "rel" if self.tx.store().has::<Link>(node) => {
                self.tx.edit::<Link>(node).rel(LinkRel::EMPTY);
            }
            "src" if self.tx.store().has::<Image>(node) => {
                self.tx.edit::<Image>(node).src(None);
            }
            "alt" if self.tx.store().has::<Image>(node) => {
                self.tx.edit::<Image>(node).alt("");
            }
            "value" if self.tx.store().has::<TextInput>(node) => {
                self.tx.edit::<TextInput>(node).value("");
            }
            "placeholder" if self.tx.store().has::<TextInput>(node) => {
                self.tx.edit::<TextInput>(node).placeholder("");
            }
            "required" if self.tx.store().has::<TextInput>(node) => {
                self.tx.edit::<TextInput>(node).required(false);
            }
            "readOnly" | "read_only" | "readonly" if self.tx.store().has::<TextInput>(node) => {
                self.tx.edit::<TextInput>(node).read_only(false);
            }
            "multiline" if self.tx.store().has::<TextInput>(node) => {
                self.tx.edit::<TextInput>(node).multiline(false);
            }
            "secureTextEntry" | "secure_text_entry" if self.tx.store().has::<TextInput>(node) => {
                self.tx.edit::<TextInput>(node).secure(false);
            }
            "disabled" | "ariaDisabled" | "aria-disabled" | "aria_disabled" => {
                self.apply_bool_attr(node, "disabled", false);
            }
            "tabIndex" | "tab-index" | "tab_index" if self.tx.store().has::<Interaction>(node) => {
                self.tx.edit::<Interaction>(node).tab_index(None);
            }
            "accessibilityLabel" | "aria-label" | "ariaLabel"
                if self.tx.store().has::<Accessibility>(node) =>
            {
                self.tx.edit::<Accessibility>(node).label("");
            }
            "accessibilityRole" | "role" if self.tx.store().has::<Accessibility>(node) => {
                self.tx.edit::<Accessibility>(node).role(None);
            }
            "ariaChecked" | "aria-checked" | "aria_checked"
                if self.tx.store().has::<Accessibility>(node) =>
            {
                self.tx.edit::<Accessibility>(node).checked(None);
            }
            "ariaSelected" | "aria-selected" | "aria_selected"
                if self.tx.store().has::<Accessibility>(node) =>
            {
                self.tx.edit::<Accessibility>(node).selected(None);
            }
            "ariaExpanded" | "aria-expanded" | "aria_expanded"
                if self.tx.store().has::<Accessibility>(node) =>
            {
                self.tx.edit::<Accessibility>(node).expanded(None);
            }
            "ariaModal" | "aria-modal" | "aria_modal"
                if self.tx.store().has::<Accessibility>(node) =>
            {
                self.tx.edit::<Accessibility>(node).modal(false);
            }
            "ariaInvalid" | "aria-invalid" | "aria_invalid"
                if self.tx.store().has::<Accessibility>(node) =>
            {
                self.tx.edit::<Accessibility>(node).invalid(false);
            }
            _ => {}
        }
    }
}

#[inline]
fn parse_tri_state(s: &str) -> Option<AriaTriState> {
    match s.trim() {
        "true" => Some(AriaTriState::True),
        "false" => Some(AriaTriState::False),
        "mixed" => Some(AriaTriState::Mixed),
        _ => None,
    }
}

