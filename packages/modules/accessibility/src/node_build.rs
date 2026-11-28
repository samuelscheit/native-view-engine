//! Single-node AccessKit [`accesskit::Node`] projection and deterministic `XXH64` content hashing.

use std::borrow::Cow;
use std::collections::HashMap;
use std::hash::Hasher;

use accesskit::{
    Action, Affine, AutoComplete, Invalid, Live, Node, NodeId as AccessKitNodeId, Orientation,
    Rect, Role, TextPosition, TextSelection, Toggled,
};
use nve_abi::InteractionEvent;
use nve_hash::Xxh64Hasher;
use nve_store::{NodeId, NodeStore};
use nve_tables::tables::{
    node_clips_overflow, Accessibility, AriaAutoComplete, AriaLive, AriaOrientation, AriaRelations,
    AriaTriState, DebugId, Image, Interaction, Link, Scroll, ScrollAxes, TextContent, TextInput,
};

use crate::action::byte_offset_to_char_index;
use crate::geometry::{A11yGeometryProvider, A11yNodeHash};
use crate::id::{encode_store_node_id, encode_text_run_id, StoreEpoch};
use crate::role::resolve_node_role;

#[inline]
fn hash_u32(h: &mut Xxh64Hasher, v: u32) {
    h.write(&v.to_le_bytes());
}

#[inline]
fn hash_u64(h: &mut Xxh64Hasher, v: u64) {
    h.write(&v.to_le_bytes());
}

#[inline]
fn hash_f64(h: &mut Xxh64Hasher, v: f64) {
    let norm = if v == 0.0 { 0.0 } else { v };
    h.write(&norm.to_bits().to_le_bytes());
}

#[inline]
fn hash_str(h: &mut Xxh64Hasher, tag: u8, s: &str) {
    h.write(&[tag]);
    hash_u64(h, s.len() as u64);
    h.write(s.as_bytes());
}

fn resolve_id_refs(
    raw: &str,
    id_map: &HashMap<String, NodeId>,
    epoch: StoreEpoch,
    out: &mut Vec<AccessKitNodeId>,
) {
    out.clear();
    for token in raw.split_ascii_whitespace() {
        if let Some(&target) = id_map.get(token) {
            out.push(encode_store_node_id(epoch, target));
        }
    }
}

/// Output of projecting a single [`NodeId`] into AccessKit form.
pub(crate) struct BuiltA11yNode {
    pub node: Node,
    pub text_run: Option<(AccessKitNodeId, Node)>,
    pub hash: A11yNodeHash,
}

/// Projects `id` into an [`accesskit::Node`] (plus optional synthetic `Role::TextRun` child for
/// `TextInput`) and computes its deterministic [`A11yNodeHash`].
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_a11y_node(
    store: &NodeStore,
    geo: &dyn A11yGeometryProvider,
    epoch: StoreEpoch,
    id: NodeId,
    window_title: &str,
    scale_factor: f32,
    id_map: &HashMap<String, NodeId>,
    child_ids: &[AccessKitNodeId],
    rel_scratch: &mut Vec<AccessKitNodeId>,
) -> BuiltA11yNode {
    let mut h = Xxh64Hasher::new(0xA117_0001);
    let role = resolve_node_role(store, id);
    let mut node = Node::new(role);
    hash_u32(&mut h, role as u32);

    // 1. Bounds, HiDPI root transform, and O(1) scroll transform.
    let is_scroll = store.has::<Scroll>(id);
    let (scroll_x, scroll_y) = if is_scroll {
        let (sx, sy) = geo.scroll_offset(id);
        (
            if sx.is_finite() { sx.max(0.0) } else { 0.0 },
            if sy.is_finite() { sy.max(0.0) } else { 0.0 },
        )
    } else {
        (0.0, 0.0)
    };
    let (sticky_x, sticky_y) = geo.sticky_delta(id);
    let sx_delta = if sticky_x.is_finite() { sticky_x } else { 0.0 };
    let sy_delta = if sticky_y.is_finite() { sticky_y } else { 0.0 };

    let mut node_bounds: Option<Rect> = None;
    if let Some(r) = geo.layout_rect(id)
        && r.x.is_finite()
        && r.y.is_finite()
        && r.width.is_finite()
        && r.height.is_finite()
    {
        // Scroll containers carry `Affine::translate((-scroll_x, -scroll_y))` so scrolling only
        // mutates the scroll node itself (not its 1,000 children); compensating the scroll node's
        // own bounds by `(+scroll_x, +scroll_y)` keeps its transformed viewport rect stationary.
        let x0 = f64::from(r.x + sx_delta + scroll_x);
        let y0 = f64::from(r.y + sy_delta + scroll_y);
        let x1 = x0 + f64::from(r.width.max(0.0));
        let y1 = y0 + f64::from(r.height.max(0.0));
        let bounds = Rect { x0, y0, x1, y1 };
        node.set_bounds(bounds);
        node_bounds = Some(bounds);
        h.write(&[1]);
        hash_f64(&mut h, x0);
        hash_f64(&mut h, y0);
        hash_f64(&mut h, x1);
        hash_f64(&mut h, y1);
    } else {
        h.write(&[0]);
    }

    let safe_scale = if scale_factor.is_finite() && scale_factor > 0.0 {
        f64::from(scale_factor)
    } else {
        1.0
    };
    if id == NodeId::ROOT && (safe_scale - 1.0).abs() > 1e-6 {
        let tf = if is_scroll && (scroll_x != 0.0 || scroll_y != 0.0) {
            Affine::scale(safe_scale) * Affine::translate((-f64::from(scroll_x), -f64::from(scroll_y)))
        } else {
            Affine::scale(safe_scale)
        };
        node.set_transform(tf);
        h.write(&[2]);
        for c in tf.as_coeffs() {
            hash_f64(&mut h, c);
        }
    } else if is_scroll && (scroll_x != 0.0 || scroll_y != 0.0) {
        let tf = Affine::translate((-f64::from(scroll_x), -f64::from(scroll_y)));
        node.set_transform(tf);
        h.write(&[3]);
        hash_f64(&mut h, f64::from(scroll_x));
        hash_f64(&mut h, f64::from(scroll_y));
    }

    if node_clips_overflow(store, id) {
        node.set_clips_children();
        h.write(&[4]);
    }

    // 2. Scroll metadata & scroll actions.
    if let Some(scroll) = store.try_row::<Scroll>(id) {
        node.set_clips_children();
        let axes = scroll.axes();
        let (vw, vh) = geo
            .layout_rect(id)
            .map_or((0.0f32, 0.0f32), |r| (r.width.max(0.0), r.height.max(0.0)));
        let (cw, ch) = geo.scroll_content_size(id).unwrap_or((vw, vh));
        let max_x = f64::from((cw - vw).max(0.0));
        let max_y = f64::from((ch - vh).max(0.0));
        let can_x = matches!(axes, ScrollAxes::Horizontal | ScrollAxes::Both);
        let can_y = matches!(axes, ScrollAxes::Vertical | ScrollAxes::Both);
        if can_x {
            node.set_scroll_x(f64::from(scroll_x));
            node.set_scroll_x_min(0.0);
            node.set_scroll_x_max(max_x);
            node.add_action(Action::ScrollLeft);
            node.add_action(Action::ScrollRight);
            node.add_action(Action::SetScrollOffset);
            h.write(&[5]);
            hash_f64(&mut h, f64::from(scroll_x));
            hash_f64(&mut h, max_x);
        }
        if can_y {
            node.set_scroll_y(f64::from(scroll_y));
            node.set_scroll_y_min(0.0);
            node.set_scroll_y_max(max_y);
            node.add_action(Action::ScrollUp);
            node.add_action(Action::ScrollDown);
            node.add_action(Action::SetScrollOffset);
            h.write(&[6]);
            hash_f64(&mut h, f64::from(scroll_y));
            hash_f64(&mut h, max_y);
        }
    }

    // 3. Window root label, Accessibility attributes, DebugId, Link, Image, TextContent.
    let mut explicit_label: Option<&str> = None;
    let mut explicit_value_text: Option<&str> = None;
    if let Some(a11y) = store.try_row::<Accessibility>(id) {
        let lbl = a11y.label();
        if !lbl.is_empty() {
            explicit_label = Some(lbl);
        }
        let hint = a11y.hint();
        if !hint.is_empty() {
            node.set_description(hint);
            hash_str(&mut h, 10, hint);
        }
        let vt = a11y.value_text();
        if !vt.is_empty() {
            explicit_value_text = Some(vt);
        }
        if let Some(raw_lvl) = a11y.level()
            && raw_lvl > 0
        {
            // AccessKit `level` is 0-based while ARIA `aria-level` is 1-based (`1..=6`).
            let lvl = usize::from(raw_lvl).saturating_sub(1);
            node.set_level(lvl);
            hash_u32(&mut h, (lvl as u32) + 1);
        }
        if a11y.modal() {
            node.set_modal();
            h.write(&[11]);
        }
        if a11y.invalid() {
            node.set_invalid(Invalid::True);
            h.write(&[12]);
        }
        if a11y.multiselectable() {
            node.set_multiselectable();
            h.write(&[13]);
        }
        if a11y.busy() {
            node.set_busy();
            h.write(&[14]);
        }
        match a11y.live() {
            None => {}
            Some(AriaLive::Off) => {
                node.set_live(Live::Off);
                h.write(&[15, 0]);
            }
            Some(AriaLive::Polite) => {
                node.set_live(Live::Polite);
                h.write(&[15, 1]);
            }
            Some(AriaLive::Assertive) => {
                node.set_live(Live::Assertive);
                h.write(&[15, 2]);
            }
        }
        match a11y.orientation() {
            None => {}
            Some(AriaOrientation::Horizontal) => {
                node.set_orientation(Orientation::Horizontal);
                h.write(&[16, 0]);
            }
            Some(AriaOrientation::Vertical) => {
                node.set_orientation(Orientation::Vertical);
                h.write(&[16, 1]);
            }
        }
        match a11y.autocomplete() {
            None | Some(AriaAutoComplete::None) => {}
            Some(AriaAutoComplete::Inline) => {
                node.set_auto_complete(AutoComplete::Inline);
                h.write(&[17, 1]);
            }
            Some(AriaAutoComplete::List) => {
                node.set_auto_complete(AutoComplete::List);
                h.write(&[17, 2]);
            }
        }
        match a11y.checked().or_else(|| a11y.pressed()) {
            None => {}
            Some(AriaTriState::False) => {
                node.set_toggled(Toggled::False);
                h.write(&[18, 0]);
            }
            Some(AriaTriState::True) => {
                node.set_toggled(Toggled::True);
                h.write(&[18, 1]);
            }
            Some(AriaTriState::Mixed) => {
                node.set_toggled(Toggled::Mixed);
                h.write(&[18, 2]);
            }
        }
        match a11y.selected() {
            None => {}
            Some(AriaTriState::False) => {
                node.set_selected(false);
                h.write(&[19, 0]);
            }
            Some(AriaTriState::True | AriaTriState::Mixed) => {
                node.set_selected(true);
                h.write(&[19, 1]);
            }
        }
        match a11y.expanded() {
            None => {}
            Some(AriaTriState::False) => {
                node.set_expanded(false);
                node.add_action(Action::Expand);
                h.write(&[20, 0]);
            }
            Some(AriaTriState::True | AriaTriState::Mixed) => {
                node.set_expanded(true);
                node.add_action(Action::Collapse);
                h.write(&[20, 1]);
            }
        }
        if let Some(raw_v) = a11y.value_now()
            && raw_v.is_finite()
        {
            let v = f64::from(raw_v);
            node.set_numeric_value(v);
            h.write(&[21]);
            hash_f64(&mut h, v);
        }
        if let Some(raw_v) = a11y.value_min()
            && raw_v.is_finite()
        {
            let v = f64::from(raw_v);
            node.set_min_numeric_value(v);
            h.write(&[22]);
            hash_f64(&mut h, v);
        }
        if let Some(raw_v) = a11y.value_max()
            && raw_v.is_finite()
        {
            let v = f64::from(raw_v);
            node.set_max_numeric_value(v);
            h.write(&[23]);
            hash_f64(&mut h, v);
        }
        if role == Role::Slider {
            node.add_action(Action::Increment);
            node.add_action(Action::Decrement);
            node.add_action(Action::SetValue);
        }
        let rel_id = a11y.relations();
        if !rel_id.is_none()
            && let Some(rel) = store.resource::<AriaRelations>(rel_id)
        {
            let controls = rel.controls();
            if !controls.is_empty() {
                resolve_id_refs(controls, id_map, epoch, rel_scratch);
                if !rel_scratch.is_empty() {
                    node.set_controls(rel_scratch.as_slice());
                    h.write(&[24]);
                    for r in rel_scratch.iter() {
                        hash_u64(&mut h, r.0);
                    }
                }
            }
            let labelled_by = rel.labelled_by();
            if !labelled_by.is_empty() {
                resolve_id_refs(labelled_by, id_map, epoch, rel_scratch);
                if !rel_scratch.is_empty() {
                    node.set_labelled_by(rel_scratch.as_slice());
                    h.write(&[25]);
                    for r in rel_scratch.iter() {
                        hash_u64(&mut h, r.0);
                    }
                }
            }
            let described_by = rel.described_by();
            if !described_by.is_empty() {
                resolve_id_refs(described_by, id_map, epoch, rel_scratch);
                if !rel_scratch.is_empty() {
                    node.set_described_by(rel_scratch.as_slice());
                    h.write(&[26]);
                    for r in rel_scratch.iter() {
                        hash_u64(&mut h, r.0);
                    }
                }
            }
            let active_desc = rel.active_descendant();
            if !active_desc.is_empty()
                && let Some(&target) = id_map.get(active_desc.trim())
            {
                let ak_id = encode_store_node_id(epoch, target);
                node.set_active_descendant(ak_id);
                h.write(&[27]);
                hash_u64(&mut h, ak_id.0);
            }
        }
    }

    if let Some(dbg) = store.try_row::<DebugId>(id) {
        let dom_id = dbg.id();
        let test_id = dbg.test_id();
        if !dom_id.is_empty() {
            node.set_author_id(dom_id);
            hash_str(&mut h, 28, dom_id);
        } else if !test_id.is_empty() {
            node.set_author_id(test_id);
            hash_str(&mut h, 28, test_id);
        }
    }

    if let Some(link) = store.try_row::<Link>(id) {
        let href = link.href();
        if !href.is_empty() {
            node.set_url(href);
            hash_str(&mut h, 29, href);
        }
        node.add_action(Action::Click);
    }

    if let Some(img) = store.try_row::<Image>(id) {
        let alt = img.alt();
        if explicit_label.is_none() && !alt.is_empty() {
            explicit_label = Some(alt);
        }
    }

    if id == NodeId::ROOT && explicit_label.is_none() && !window_title.is_empty() {
        explicit_label = Some(window_title);
    }

    if let Some(lbl) = explicit_label {
        node.set_label(lbl);
        hash_str(&mut h, 30, lbl);
    }

    // 4. Interaction flags & actions.
    let mut is_disabled = false;
    if let Some(inter) = store.try_row::<Interaction>(id) {
        if inter.disabled() {
            is_disabled = true;
            node.set_disabled();
            h.write(&[31]);
        }
        let listeners = inter.listeners();
        if !is_disabled && listeners.contains(InteractionEvent::Press) {
            node.add_action(Action::Click);
        }
        if !is_disabled
            && (inter.focusable()
                || inter.tab_index().is_some()
                || listeners.contains(InteractionEvent::Focus)
                || listeners.contains(InteractionEvent::Blur))
        {
            node.add_action(Action::Focus);
            node.add_action(Action::Blur);
        }
    }
    if id != NodeId::ROOT {
        node.add_action(Action::ScrollIntoView);
    }

    // 5. TextContent / TextInput value, masking, synthetic TextRun, and TextSelection.
    let mut text_run: Option<(AccessKitNodeId, Node)> = None;
    if let Some(ti) = store.try_row::<TextInput>(id) {
        let ph = ti.placeholder();
        if !ph.is_empty() {
            node.set_placeholder(ph);
            hash_str(&mut h, 32, ph);
        }
        if ti.read_only() {
            node.set_read_only();
            h.write(&[33]);
        }
        if ti.required() {
            node.set_required();
            h.write(&[34]);
        }
        if !is_disabled {
            node.add_action(Action::Focus);
            node.add_action(Action::Blur);
            node.add_action(Action::SetTextSelection);
            if !ti.read_only() {
                node.add_action(Action::SetValue);
                node.add_action(Action::ReplaceSelectedText);
            }
        }
        let raw_val = geo
            .text_input_value(id)
            .unwrap_or_else(|| Cow::Borrowed(ti.value()));
        let is_password = role == Role::PasswordInput;
        let masked: String;
        let exposed_val: &str = if is_password {
            masked = "•".repeat(raw_val.chars().count());
            masked.as_str()
        } else {
            raw_val.as_ref()
        };
        node.set_value(exposed_val);
        hash_str(&mut h, 35, exposed_val);

        // Emit synthetic `Role::TextRun` child with exact UTF-8 byte `character_lengths` so
        // screen readers & magnifiers can navigate characters and set text selections.
        let run_id = encode_text_run_id(epoch, id);
        let mut run = Node::new(Role::TextRun);
        run.set_value(exposed_val);
        let char_lens: Vec<u8> = exposed_val
            .chars()
            .map(|c| c.len_utf8() as u8)
            .collect();
        run.set_character_lengths(char_lens);
        if let Some(b) = node_bounds {
            run.set_bounds(b);
        }
        if let Some((anchor_byte, focus_byte)) = geo.text_selection_bytes(id) {
            // `geo.text_selection_bytes` is indexed into `raw_val`; char indices match 1:1 with
            // masked bullets for `PasswordInput`.
            let anchor_char = byte_offset_to_char_index(raw_val.as_ref(), anchor_byte);
            let focus_char = byte_offset_to_char_index(raw_val.as_ref(), focus_byte);
            node.set_text_selection(TextSelection {
                anchor: TextPosition {
                    node: run_id,
                    character_index: anchor_char,
                },
                focus: TextPosition {
                    node: run_id,
                    character_index: focus_char,
                },
            });
            h.write(&[36]);
            hash_u64(&mut h, anchor_char as u64);
            hash_u64(&mut h, focus_char as u64);
        }
        text_run = Some((run_id, run));
    } else if let Some(vt) = explicit_value_text {
        node.set_value(vt);
        hash_str(&mut h, 37, vt);
    } else if let Some(tc) = store.try_row::<TextContent>(id) {
        let txt = tc.text();
        if !txt.is_empty() {
            if role == Role::Label {
                // AccessKit contract: `Role::Label` text content lives in `Node::value`.
                node.set_value(txt);
                hash_str(&mut h, 38, txt);
            } else if explicit_label.is_none() {
                node.set_label(txt);
                hash_str(&mut h, 30, txt);
            }
        }
    }

    // 6. Children (plus synthetic TextRun child if this node is a TextInput).
    if let Some((run_id, _)) = &text_run {
        let mut combined = Vec::with_capacity(child_ids.len() + 1);
        combined.extend_from_slice(child_ids);
        combined.push(*run_id);
        node.set_children(combined.as_slice());
        h.write(&[40]);
        for c in &combined {
            hash_u64(&mut h, c.0);
        }
    } else if !child_ids.is_empty() {
        node.set_children(child_ids);
        h.write(&[41]);
        for c in child_ids {
            hash_u64(&mut h, c.0);
        }
    }

    // Hash final action bitset by sampling canonical actions.
    let mut act_bits = 0u32;
    const TRACKED_ACTIONS: [Action; 16] = [
        Action::Click,
        Action::Focus,
        Action::Blur,
        Action::Expand,
        Action::Collapse,
        Action::Increment,
        Action::Decrement,
        Action::SetValue,
        Action::ReplaceSelectedText,
        Action::SetTextSelection,
        Action::ScrollIntoView,
        Action::ScrollUp,
        Action::ScrollDown,
        Action::ScrollLeft,
        Action::ScrollRight,
        Action::SetScrollOffset,
    ];
    for (bit, act) in TRACKED_ACTIONS.iter().enumerate() {
        if node.supports_action(*act) {
            act_bits |= 1u32 << bit;
        }
    }
    hash_u32(&mut h, act_bits);

    BuiltA11yNode {
        node,
        text_run,
        hash: A11yNodeHash::new(h.finish()),
    }
}
