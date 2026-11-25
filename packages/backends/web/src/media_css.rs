//! Zero-serialization `@media` CSS compilation for [`MediaQuery`] visibility (`show_on`) and
//! [`ResponsiveRule`] style overrides.
//!
//! Accepts strictly typed Rust structs/slices directly from `nve_components::media` with zero
//! JSON, zero wire serialization, and 0% CSS injection surface.

use core::fmt::Write as _;

use nve_abi::hash::HashMap;
use nve_abi::num::{write_i64, write_u64};
use nve_abi::{EdgeInsets, FlexAlign, FlexDirection, FlexJustify, NodeId};
use nve_components::media::{MediaQuery, ResponsiveRule};
use nve_components::style_patch::StylePatch;

use crate::css::{
    canonical_f32, write_compact_color, write_compact_dim, write_compact_edges, write_compact_num,
    write_compact_px,
};
use crate::WebBackend;

const FNV1A_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const FNV1A_PRIME: u64 = 0x0000_0100_0000_01b3;

/// One compiled, content-addressed CSS `@media` rule (`@media <condition>{.<class_name>{<decls>}}`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledMediaRule {
    pub class_name: String,
    pub condition: String,
    pub decls: String,
}

impl CompiledMediaRule {
    /// Appends `@media {condition}{.{class_name}{decls}}` into `out`.
    pub fn write_css_rule_into(&self, out: &mut String) {
        out.push_str("@media ");
        out.push_str(&self.condition);
        out.push_str("{.");
        out.push_str(&self.class_name);
        out.push('{');
        out.push_str(&self.decls);
        out.push_str("}}");
    }

    /// Formats the complete CSS `@media` rule string.
    pub fn to_css_rule(&self) -> String {
        let mut out = String::with_capacity(self.condition.len() + self.class_name.len() + self.decls.len() + 12);
        self.write_css_rule_into(&mut out);
        out
    }
}

#[inline]
fn fnv1a_bytes(mut hash: u64, bytes: &[u8]) -> u64 {
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(FNV1A_PRIME);
    }
    hash
}

/// Formats a deterministic content-addressed class token `nve-m-<8hex>` for `(condition, decls)`.
pub fn format_media_class(condition: &str, decls: &str) -> String {
    let mut h = FNV1A_OFFSET_BASIS;
    h = fnv1a_bytes(h, condition.as_bytes());
    h = fnv1a_bytes(h, &[0]);
    h = fnv1a_bytes(h, decls.as_bytes());
    let h32 = (h ^ (h >> 32)) as u32;
    let mut out = String::with_capacity(14);
    let _ = write!(&mut out, "nve-m-{h32:08x}");
    out
}

/// Appends compact CSS shorthand for `padding`/`margin`, explicitly emitting `{prop}:0;` for
/// all-zero overrides so a responsive patch can reset a non-zero base value.
fn write_override_edges(prop: &str, e: EdgeInsets, out: &mut String) {
    let t = canonical_f32(e.top);
    let r = canonical_f32(e.right);
    let b = canonical_f32(e.bottom);
    let l = canonical_f32(e.left);
    if t == 0.0 && r == 0.0 && b == 0.0 && l == 0.0 {
        out.push_str(prop);
        out.push_str(":0;");
    } else {
        write_compact_edges(prop, e, out);
    }
}

/// Emits strictly typed, compact CSS declarations for every populated field of `patch` (shared by
/// both responsive `@media` rules and interactive pseudo-state rules).
///
/// Unlike base structural emitters, default values (such as `Column`, `0`, `Auto`, or `EdgeInsets::ZERO`)
/// are emitted explicitly whenever present in the sparse patch so they override base rules cleanly.
pub fn write_style_patch_decls(patch: &StylePatch, out: &mut String) {
    if let Some(dir) = patch.flex_direction {
        out.push_str("flex-direction:");
        out.push_str(match dir {
            FlexDirection::Column => "column",
            FlexDirection::Row => "row",
            FlexDirection::ColumnReverse => "column-reverse",
            FlexDirection::RowReverse => "row-reverse",
        });
        out.push(';');
    }
    if let Some(j) = patch.justify_content {
        out.push_str("justify-content:");
        out.push_str(match j {
            FlexJustify::FlexStart => "flex-start",
            FlexJustify::Center => "center",
            FlexJustify::FlexEnd => "flex-end",
            FlexJustify::SpaceBetween => "space-between",
            FlexJustify::SpaceAround => "space-around",
            FlexJustify::SpaceEvenly => "space-evenly",
        });
        out.push(';');
    }
    if let Some(a) = patch.align_items {
        out.push_str("align-items:");
        out.push_str(match a {
            FlexAlign::Stretch => "stretch",
            FlexAlign::FlexStart => "flex-start",
            FlexAlign::Center => "center",
            FlexAlign::FlexEnd => "flex-end",
            FlexAlign::Baseline => "baseline",
        });
        out.push(';');
    }
    if let Some(fg) = patch.flex_grow {
        out.push_str("flex-grow:");
        write_compact_num(canonical_f32(fg).max(0.0), out);
        out.push(';');
    }
    if let Some(fs) = patch.flex_shrink {
        out.push_str("flex-shrink:");
        write_compact_num(canonical_f32(fs).max(0.0), out);
        out.push(';');
    }
    if let Some(gap) = patch.gap {
        out.push_str("gap:");
        write_compact_px(canonical_f32(gap).max(0.0), out);
        out.push(';');
    }

    for (prop, dim) in [
        ("width:", patch.width),
        ("height:", patch.height),
        ("min-width:", patch.min_width),
        ("max-width:", patch.max_width),
        ("min-height:", patch.min_height),
        ("max-height:", patch.max_height),
    ] {
        if let Some(d) = dim {
            out.push_str(prop);
            write_compact_dim(d, out);
            out.push(';');
        }
    }

    if let Some(pad) = patch.padding {
        write_override_edges("padding", pad, out);
    }
    if let Some(mar) = patch.margin {
        write_override_edges("margin", mar, out);
    }
    if let Some(bg) = patch.background {
        out.push_str("background-color:");
        write_compact_color(bg, out);
        out.push(';');
    }
    if let Some(tc) = patch.text_color {
        out.push_str("color:");
        write_compact_color(tc, out);
        out.push(';');
    }
    match (patch.border_width, patch.border_color) {
        (Some(bw), Some(bc)) => {
            out.push_str("border:");
            write_compact_px(canonical_f32(bw).max(0.0), out);
            out.push_str(" solid ");
            write_compact_color(bc, out);
            out.push(';');
        }
        (Some(bw), None) => {
            out.push_str("border-width:");
            write_compact_px(canonical_f32(bw).max(0.0), out);
            out.push(';');
        }
        (None, Some(bc)) => {
            out.push_str("border-color:");
            write_compact_color(bc, out);
            out.push(';');
        }
        (None, None) => {}
    }
    if let Some(br) = patch.border_radius {
        out.push_str("border-radius:");
        write_compact_px(canonical_f32(br).max(0.0), out);
        out.push(';');
    }
    if let Some(op) = patch.opacity {
        out.push_str("opacity:");
        write_compact_num(canonical_f32(op).clamp(0.0, 1.0), out);
        out.push(';');
    }
    if let Some(fs) = patch.font_size {
        out.push_str("font-size:");
        write_compact_px(canonical_f32(fs).max(1.0), out);
        out.push(';');
    }
    if let Some(z) = patch.z_index {
        out.push_str("z-index:");
        write_i64(out, i64::from(z));
        out.push(';');
    }
    if let Some(wrap) = patch.flex_wrap {
        out.push_str("flex-wrap:");
        out.push_str(wrap.as_css());
        out.push(';');
    }
    if let Some(ov) = patch.overflow {
        out.push_str("overflow:");
        out.push_str(ov.as_css());
        out.push(';');
    }
    if let Some(align) = patch.text_align {
        out.push_str("text-align:");
        out.push_str(align.as_css());
        out.push(';');
    }
    if let Some(fw) = patch.font_weight {
        out.push_str("font-weight:");
        write_u64(out, u64::from(fw.clamp(1, 1000)));
        out.push(';');
    }
    if let Some(display) = patch.display {
        out.push_str("display:");
        out.push_str(display.as_css());
        out.push(';');
    }
    if let Some(cursor) = patch.cursor {
        out.push_str("cursor:");
        out.push_str(cursor.to_css_keyword());
        out.push(';');
    }
}

/// Backwards-compatible alias for [`write_style_patch_decls`].
#[inline]
pub fn write_responsive_patch_decls(patch: &StylePatch, out: &mut String) {
    write_style_patch_decls(patch, out);
}

/// Compiles a node's `show_on` visibility query and `responsive_rules` overrides into
/// content-addressed [`CompiledMediaRule`] entries, deduplicated in declaration order.
pub fn compile_node_media_rules(
    show_on: Option<MediaQuery>,
    responsive_rules: &[ResponsiveRule],
    out: &mut Vec<CompiledMediaRule>,
) {
    let push_unique = |out: &mut Vec<CompiledMediaRule>, rule: CompiledMediaRule| {
        if !out.iter().any(|existing| existing.class_name == rule.class_name) {
            out.push(rule);
        }
    };

    if let Some(query) = show_on {
        let mut negated = Vec::new();
        query.write_negated_css_conditions(&mut negated);
        const HIDE_DECL: &str = "display:none!important;";
        for condition in negated {
            let class_name = format_media_class(&condition, HIDE_DECL);
            push_unique(
                out,
                CompiledMediaRule {
                    class_name,
                    condition,
                    decls: HIDE_DECL.to_owned(),
                },
            );
        }
    }

    for rule in responsive_rules {
        if rule.patch.is_empty() || !rule.query.is_valid() {
            continue;
        }
        let mut condition = String::new();
        if !rule.query.write_css_condition(&mut condition) {
            continue;
        }
        let mut decls = String::new();
        write_responsive_patch_decls(&rule.patch, &mut decls);
        if decls.is_empty() {
            continue;
        }
        let class_name = format_media_class(&condition, &decls);
        push_unique(
            out,
            CompiledMediaRule {
                class_name,
                condition,
                decls,
            },
        );
    }
}

impl WebBackend {
    pub(crate) fn flush_or_queue_responsive_dirty(&mut self, dirty: Vec<NodeId>) {
        if dirty.is_empty() {
            return;
        }
        #[cfg(feature = "hydrate")]
        if self.hydrating {
            self.pending_responsive_dirty.extend(dirty);
            return;
        }
        self.flush_dirty_styles(&dirty);
    }

    /// Directly attaches or clears a zero-serialization [`MediaQuery`] visibility predicate on `id`.
    pub fn set_node_show_on(&mut self, id: NodeId, query: Option<MediaQuery>) {
        let Some(node) = self.nodes.get_mut(&id) else {
            return;
        };
        if node.show_on != query {
            node.show_on = query;
            self.flush_or_queue_responsive_dirty(vec![id]);
        }
    }

    /// Directly replaces the zero-serialization [`ResponsiveRule`] list on `id`.
    pub fn set_node_responsive_rules(&mut self, id: NodeId, rules: Vec<ResponsiveRule>) {
        let Some(node) = self.nodes.get_mut(&id) else {
            return;
        };
        if node.responsive_rules != rules {
            node.responsive_rules = rules;
            self.flush_or_queue_responsive_dirty(vec![id]);
        }
    }

    /// Bulk-synchronizes `show_on` visibility queries and `responsive_rules` slices from any
    /// producer (e.g. `DioxusSurface`), clearing stale responsive metadata from nodes no longer
    /// present and flushing updated classes & `@media` rules in a single batched pass.
    pub fn sync_responsive_from<'a>(
        &mut self,
        show_on: impl IntoIterator<Item = (NodeId, MediaQuery)>,
        rules: impl IntoIterator<Item = (NodeId, &'a [ResponsiveRule])>,
    ) {
        let show_map: HashMap<NodeId, MediaQuery> = show_on.into_iter().collect();
        let rules_map: HashMap<NodeId, &'a [ResponsiveRule]> = rules.into_iter().collect();

        let mut dirty: Vec<NodeId> = Vec::new();
        for (&id, node) in &mut self.nodes {
            let mut changed = false;
            let next_show = show_map.get(&id).copied();
            if node.show_on != next_show {
                node.show_on = next_show;
                changed = true;
            }
            let next_rules = rules_map.get(&id).copied().unwrap_or(&[]);
            if node.responsive_rules.as_slice() != next_rules {
                node.responsive_rules.clear();
                node.responsive_rules.extend_from_slice(next_rules);
                changed = true;
            }
            if changed {
                dirty.push(id);
            }
        }

        if !dirty.is_empty() {
            dirty.sort_unstable();
            dirty.dedup();
            self.flush_or_queue_responsive_dirty(dirty);
        }
    }
}
