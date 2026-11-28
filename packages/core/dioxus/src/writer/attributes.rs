//! Direct zero-string-parse attribute application onto `NodeStore` word tables.
//!
//! - Matches `AttributeValue::Float`/`Int`/`Bool` directly without string formatting/reparsing.
//! - Matches `TypedStyle`, `MediaQuery`, and extensible [`TableAttr`] directly via `AttributeValue::Any`.
//! - Zero runtime CSS string parsing, zero `set_raw_prop`, zero `ComponentKind`, zero module-specific downcasts.

use dioxus_core::{AttributeValue, ElementId};
use nve_tables::input_hints::{AutofillHint, InputMode};
use nve_tables::media::MediaQuery;
use nve_tables::tables::{
    Accessibility, AriaAutoComplete, AriaLive, AriaOrientation, AriaRelations, AriaRole,
    AriaTriState, Image, ImageFit, ImageSrc, InputKind, Interaction, Link, LinkRel, LinkTarget,
    Responsive, Scroll, TextInput, Visual,
};
use nve_store::NodeId;

use crate::table_attr::TableAttr;
use crate::typed_style::{pack_media_query, TypedStyle, NVE_SHOW_ON_ATTR, NVE_TYPED_STYLE_ATTR};

use super::attributes_num::apply_numeric_attr_untracked;
use super::standalone_style::numeric_attr_layer;
use super::{StandaloneStyleVal, StoreMutationTx, StoredTypedStyle};

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
                    let dropped_cursor = if let Some(prev) = self.typed_styles.get_mut(node) {
                        if prev.ptr_eq(any) {
                            return;
                        }
                        let prev_style = prev.as_style();
                        if prev_style == ts {
                            *prev = StoredTypedStyle::from_rc(any);
                            return;
                        }
                        let dropped_cursor = prev_style.cursor.is_some() && ts.cursor.is_none();
                        prev_style.clear_diff_from(Some(ts), self.tx, node);
                        ts.apply_to_tx_with_prev(Some(prev_style), self.tx, node);
                        *prev = StoredTypedStyle::from_rc(any);
                        dropped_cursor
                    } else {
                        ts.apply_to_tx_with_prev(None, self.tx, node);
                        self.typed_styles
                            .insert(node, StoredTypedStyle::from_rc(any));
                        false
                    };
                    if dropped_cursor {
                        self.restore_tag_default_cursor(node);
                    }
                    self.replay_standalone_styles(node, nve_tables::STYLE_LAYER_ALL);
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
                    let pool = &mut *self.buffer_pool;
                    if let Some(list) =
                        self.attached_tables.get_or_insert_with(node, || pool.table_list())
                    {
                        if let Some((_, prev)) = list.iter_mut().find(|(k, _)| *k == name) {
                            if !prev.same_writer_kind(table_attr) {
                                prev.clear_from(self.tx, node);
                            }
                            *prev = table_attr.clone();
                        } else {
                            list.push((name, table_attr.clone()));
                        }
                    }
                    table_attr.write_to(self.tx, node);
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
    fn apply_numeric_attr(&mut self, node: NodeId, name: &'static str, v: f64) {
        if apply_numeric_attr_untracked(self.tx, node, name, v) {
            self.finish_standalone_style(node, name, StandaloneStyleVal::Float(v));
        }
    }

    #[inline]
    fn apply_int_attr(&mut self, node: NodeId, name: &'static str, v: i64) {
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
            "aria-level" | "ariaLevel" | "aria_level" => {
                let lvl = (v > 0).then(|| v.min(i64::from(u8::MAX)) as u8);
                self.tx.edit::<Accessibility>(node).level(lvl);
            }
            "requestedScrollSeq" | "requested-scroll-seq" | "requested_scroll_seq" => {
                let seq = if v >= 0 {
                    v.min(i64::from(u32::MAX)) as u32
                } else {
                    0
                };
                self.tx.edit::<Scroll>(node).requested_scroll_seq(seq);
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
            "ariaPressed" | "aria-pressed" | "aria_pressed" => {
                self.tx.edit::<Accessibility>(node).pressed(Some(if b {
                    AriaTriState::True
                } else {
                    AriaTriState::False
                }));
            }
            "ariaHidden" | "aria-hidden" | "aria_hidden" => {
                self.tx.edit::<Accessibility>(node).hidden(b);
            }
            "ariaBusy" | "aria-busy" | "aria_busy" => {
                self.tx.edit::<Accessibility>(node).busy(b);
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
        if self.apply_identity_attr(node, name, Some(value)) {
            return;
        }
        match name {
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
                let hint = AutofillHint::from_whatwg_token(value);
                self.tx.edit::<TextInput>(node).autofill(hint);
            }
            "inputmode" | "inputMode" | "input_mode" => {
                let mode = InputMode::from_whatwg_token(value).unwrap_or(InputMode::Text);
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
            "minlength" => {
                if let Ok(i) = value.trim().parse::<i64>() {
                    let len = (i >= 0).then(|| i.min(i64::from(u16::MAX)) as u16);
                    self.tx.edit::<TextInput>(node).min_length(len);
                }
            }
            "maxlength" => {
                if let Ok(i) = value.trim().parse::<i64>() {
                    let len = (i >= 0).then(|| i.min(i64::from(u16::MAX)) as u16);
                    self.tx.edit::<TextInput>(node).max_length(len);
                }
            }
            "tabIndex" | "tab-index" | "tab_index" => {
                if let Ok(i) = value.trim().parse::<i64>() {
                    self.tx
                        .edit::<Interaction>(node)
                        .tab_index(Some(i.clamp(i16::MIN as i64, i16::MAX as i64) as i16));
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
            "ariaPressed" | "aria-pressed" | "aria_pressed" => {
                self.tx
                    .edit::<Accessibility>(node)
                    .pressed(parse_tri_state(value));
            }
            "ariaValueText" | "aria-valuetext" | "aria_valuetext" => {
                self.tx.edit::<Accessibility>(node).value_text(value);
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
            | "aria_activedescendant" => self.set_aria_relation(node, name, value),
            "ariaModal" | "aria-modal" | "aria_modal" | "ariaInvalid" | "aria-invalid"
            | "aria_invalid" | "ariaDisabled" | "aria-disabled" | "aria_disabled"
            | "ariaMultiSelectable" | "aria-multiselectable" | "aria_multiselectable"
            | "ariaHidden" | "aria-hidden" | "aria_hidden" | "ariaBusy" | "aria-busy"
            | "aria_busy" => {
                let b = matches!(value.trim(), "true" | "1");
                self.apply_bool_attr(node, name, b);
            }
            "ariaValueNow" | "aria-valuenow" | "aria_valuenow" | "ariaValueMin"
            | "aria-valuemin" | "aria_valuemin" | "ariaValueMax" | "aria-valuemax"
            | "aria_valuemax" => {
                if let Ok(v) = value.trim().parse::<f64>() {
                    let _ = apply_numeric_attr_untracked(self.tx, node, name, v);
                }
            }
            _ => {}
        }
    }

    fn clear_attr(&mut self, node: NodeId, name: &str) {
        if self.remove_standalone_style(node, name) {
            let layer = numeric_attr_layer(name);
            debug_assert!(
                layer != 0,
                "removed standalone numeric style `{name}` must have a non-zero layer"
            );
            self.rebuild_style_layers_mask(node, layer);
            return;
        }
        if let Some(list) = self.attached_tables.get_mut(node)
            && let Some(pos) = list.iter().position(|(k, _)| *k == name)
        {
            let (_, prev) = list.swap_remove(pos);
            let empty = list.is_empty();
            prev.clear_from(self.tx, node);
            if empty && let Some(list) = self.attached_tables.remove(node) {
                self.buffer_pool.recycle_table_list(list);
            }
            return;
        }
        if self.apply_identity_attr(node, name, None) {
            return;
        }
        match name {
            NVE_TYPED_STYLE_ATTR => {
                if let Some(prev) = self.typed_styles.remove(node) {
                    let prev_style = prev.as_style();
                    let dropped_cursor = prev_style.cursor.is_some();
                    prev_style.clear_diff_from(None, self.tx, node);
                    if dropped_cursor {
                        self.restore_tag_default_cursor(node);
                    }
                }
                self.replay_standalone_styles(node, nve_tables::STYLE_LAYER_ALL);
            }
            NVE_SHOW_ON_ATTR | NVE_SHOW_ON_KEBAB_ATTR => {
                if self.tx.store().has::<Responsive>(node) {
                    self.tx
                        .edit::<Responsive>(node)
                        .show_on_widths(0)
                        .show_on_env(0);
                }
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
            "requestedScrollX" | "requested-scroll-x" | "requested_scroll_x"
                if self.tx.store().has::<Scroll>(node) =>
            {
                self.tx.edit::<Scroll>(node).requested_scroll_x(None);
            }
            "requestedScrollY" | "requested-scroll-y" | "requested_scroll_y"
                if self.tx.store().has::<Scroll>(node) =>
            {
                self.tx.edit::<Scroll>(node).requested_scroll_y(None);
            }
            "requestedScrollSeq" | "requested-scroll-seq" | "requested_scroll_seq"
                if self.tx.store().has::<Scroll>(node) =>
            {
                self.tx.edit::<Scroll>(node).requested_scroll_seq(0);
            }
            "ariaPressed" | "aria-pressed" | "aria_pressed"
                if self.tx.store().has::<Accessibility>(node) =>
            {
                self.tx.edit::<Accessibility>(node).pressed(None);
            }
            "ariaHidden" | "aria-hidden" | "aria_hidden"
                if self.tx.store().has::<Accessibility>(node) =>
            {
                self.tx.edit::<Accessibility>(node).hidden(false);
            }
            "ariaBusy" | "aria-busy" | "aria_busy"
                if self.tx.store().has::<Accessibility>(node) =>
            {
                self.tx.edit::<Accessibility>(node).busy(false);
            }
            "ariaValueText" | "aria-valuetext" | "aria_valuetext"
                if self.tx.store().has::<Accessibility>(node) =>
            {
                self.tx.edit::<Accessibility>(node).value_text("");
            }
            "aria-level" | "ariaLevel" | "aria_level"
                if self.tx.store().has::<Accessibility>(node) =>
            {
                self.tx.edit::<Accessibility>(node).level(None);
            }
            "ariaInvalid" | "aria-invalid" | "aria_invalid"
                if self.tx.store().has::<Accessibility>(node) =>
            {
                self.tx.edit::<Accessibility>(node).invalid(false);
            }
            "ariaControls" | "aria-controls" | "aria_controls" | "ariaLabelledBy"
            | "aria-labelledby" | "aria_labelledby" | "ariaDescribedBy" | "aria-describedby"
            | "aria_describedby" | "ariaActiveDescendant" | "aria-activedescendant"
            | "aria_activedescendant"
                if self.tx.store().has::<Accessibility>(node) =>
            {
                self.set_aria_relation(node, name, "");
            }
            _ => {}
        }
    }

    fn set_aria_relation(&mut self, node: NodeId, name: &str, value: &str) {
        let mut rel = self
            .tx
            .store()
            .try_row::<Accessibility>(node)
            .and_then(|a| {
                let r = a.relations();
                (!r.is_none())
                    .then(|| self.tx.store().resource::<AriaRelations>(r).map(|v| v.to_owned()))
                    .flatten()
            })
            .unwrap_or_default();
        match name {
            "ariaControls" | "aria-controls" | "aria_controls" => rel.controls = value.to_owned(),
            "ariaLabelledBy" | "aria-labelledby" | "aria_labelledby" => {
                rel.labelled_by = value.to_owned();
            }
            "ariaDescribedBy" | "aria-describedby" | "aria_describedby" => {
                rel.described_by = value.to_owned();
            }
            _ => rel.active_descendant = value.to_owned(),
        }
        let next = (!rel.controls.is_empty()
            || !rel.labelled_by.is_empty()
            || !rel.described_by.is_empty()
            || !rel.active_descendant.is_empty())
            .then_some(rel);
        self.tx.edit::<Accessibility>(node).relations(next);
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

