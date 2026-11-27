//! Zero-serialization `@media` CSS compilation for [`MediaQuery`] visibility (`show_on`) and
//! [`ResponsiveRule`] style overrides.
//!
//! Accepts strictly typed Rust structs/slices directly from `nve_components::media` with zero
//! JSON, zero wire serialization, and 0% CSS injection surface.

use std::fmt::Write as _;

use nve_abi::num::{write_i64, write_u64};
use nve_abi::{ColorScheme, EdgeInsets, FlexAlign, FlexDirection, FlexJustify};
use nve_components::layout_ext::GridTrack;
use nve_components::media::{MediaPointer, MediaQuery, ResponsiveRule};
use nve_components::style_patch::StylePatch;

/// Formats [`ColorScheme`] as a CSS `prefers-color-scheme` keyword.
#[inline]
pub const fn color_scheme_as_css(scheme: ColorScheme) -> &'static str {
    match scheme {
        ColorScheme::Light => "light",
        ColorScheme::Dark => "dark",
    }
}

/// Formats the complementary CSS `prefers-color-scheme` keyword for De Morgan negation.
#[inline]
pub const fn color_scheme_negated_css(scheme: ColorScheme) -> &'static str {
    match scheme {
        ColorScheme::Light => "dark",
        ColorScheme::Dark => "light",
    }
}

/// Formats [`MediaPointer`] as a CSS `pointer` keyword.
#[inline]
pub const fn media_pointer_as_css(ptr: MediaPointer) -> &'static str {
    match ptr {
        MediaPointer::Fine => "fine",
        MediaPointer::Coarse => "coarse",
    }
}

/// Formats the complementary CSS `pointer` keyword for De Morgan negation.
#[inline]
pub const fn media_pointer_negated_css(ptr: MediaPointer) -> &'static str {
    match ptr {
        MediaPointer::Fine => "coarse",
        MediaPointer::Coarse => "fine",
    }
}

/// Writes a deterministic, injection-free CSS `@media` condition for `query` into `out`.
pub fn write_media_css_condition(query: &MediaQuery, out: &mut String) -> bool {
    if !query.is_valid() || query.is_empty() {
        return false;
    }
    let mut first = true;
    let mut push_sep = |buf: &mut String| {
        if !first {
            buf.push_str(" and ");
        }
        first = false;
    };

    if let Some(min) = query.min_width_opt() {
        push_sep(out);
        let _ = write!(out, "(min-width:{min}px)");
    }
    if let Some(max) = query.max_width_opt() {
        push_sep(out);
        let _ = write!(out, "(max-width:{max}.98px)");
    }
    if let Some(scheme) = query.color_scheme_opt() {
        push_sep(out);
        let _ = write!(out, "(prefers-color-scheme:{})", color_scheme_as_css(scheme));
    }
    if let Some(rm) = query.reduced_motion_opt() {
        push_sep(out);
        let kw = if rm { "reduce" } else { "no-preference" };
        let _ = write!(out, "(prefers-reduced-motion:{kw})");
    }
    if let Some(ptr) = query.pointer_opt() {
        push_sep(out);
        let _ = write!(out, "(pointer:{})", media_pointer_as_css(ptr));
    }
    !first
}

/// Emits exact complementary CSS `@media` conditions that hide a node whenever `query` does
/// NOT match (De Morgan negation: `!(A && B)` -> `[!A, !B]`).
pub fn write_negated_media_css_conditions(query: &MediaQuery, out: &mut Vec<String>) {
    if !query.is_valid() {
        out.push("all".to_owned());
        return;
    }
    if let Some(min) = query.min_width_opt()
        && min > 0
    {
        out.push(format!("(max-width:{}.98px)", min - 1));
    }
    if let Some(max) = query.max_width_opt() {
        out.push(format!("(min-width:{}px)", u32::from(max) + 1));
    }
    if let Some(scheme) = query.color_scheme_opt() {
        out.push(format!(
            "(prefers-color-scheme:{})",
            color_scheme_negated_css(scheme)
        ));
    }
    if let Some(rm) = query.reduced_motion_opt() {
        let kw = if rm { "no-preference" } else { "reduce" };
        out.push(format!("(prefers-reduced-motion:{kw})"));
    }
    if let Some(ptr) = query.pointer_opt() {
        out.push(format!("(pointer:{})", media_pointer_negated_css(ptr)));
    }
}

use crate::css::{
    canonical_f32, format_content_class, write_compact_color, write_compact_dim,
    write_compact_edges, write_compact_num, write_compact_px,
};

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

/// Formats a deterministic content-addressed class token `nve-m-<8hex>` for `(condition, decls)`.
pub fn format_media_class(condition: &str, decls: &str) -> String {
    format_content_class("nve-m-", &[condition.as_bytes(), &[0], decls.as_bytes()])
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
        out.push_str(prop);
        out.push(':');
        write_compact_edges(e, out);
        out.push(';');
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
    if let Some(ref fill) = patch.background_fill {
        fill.write_css_decl_into(true, out);
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
    if let Some(fb) = patch.flex_basis {
        out.push_str("flex-basis:");
        write_compact_dim(fb, out);
        out.push(';');
    }
    if let Some(ratio) = patch.aspect_ratio
        && ratio.is_finite()
        && ratio > 0.0
    {
        out.push_str("aspect-ratio:");
        write_compact_num(ratio, out);
        out.push(';');
    }
    if let Some(pos) = patch.position {
        out.push_str("position:");
        out.push_str(pos.as_css());
        out.push(';');
    }
    for (prop, dim) in [
        ("top:", patch.inset_top),
        ("right:", patch.inset_right),
        ("bottom:", patch.inset_bottom),
        ("left:", patch.inset_left),
    ] {
        if let Some(d) = dim {
            out.push_str(prop);
            write_compact_dim(d, out);
            out.push(';');
        }
    }
    if let Some(cg) = patch.column_gap {
        out.push_str("column-gap:");
        write_compact_px(canonical_f32(cg).max(0.0), out);
        out.push(';');
    }
    if let Some(rg) = patch.row_gap {
        out.push_str("row-gap:");
        write_compact_px(canonical_f32(rg).max(0.0), out);
        out.push(';');
    }
    if let Some(ref cols) = patch.grid_template_columns
        && !cols.is_empty()
    {
        out.push_str("grid-template-columns:");
        GridTrack::write_css_list_into(cols, out);
        out.push(';');
    }
    if let Some(ref rows) = patch.grid_template_rows
        && !rows.is_empty()
    {
        out.push_str("grid-template-rows:");
        GridTrack::write_css_list_into(rows, out);
        out.push(';');
    }
    if let Some(gc) = patch.grid_column {
        out.push_str("grid-column:");
        gc.write_css_into(out);
        out.push(';');
    }
    if let Some(gr) = patch.grid_row {
        out.push_str("grid-row:");
        gr.write_css_into(out);
        out.push(';');
    }
    if let Some(dir) = patch.direction {
        out.push_str("direction:");
        out.push_str(dir.as_html_dir());
        out.push(';');
    }
    if let Some(v) = patch.padding_inline_start {
        out.push_str("padding-inline-start:");
        write_compact_px(canonical_f32(v).max(0.0), out);
        out.push(';');
    }
    if let Some(v) = patch.padding_inline_end {
        out.push_str("padding-inline-end:");
        write_compact_px(canonical_f32(v).max(0.0), out);
        out.push(';');
    }
    if let Some(v) = patch.margin_inline_start {
        out.push_str("margin-inline-start:");
        write_compact_px(canonical_f32(v), out);
        out.push(';');
    }
    if let Some(v) = patch.margin_inline_end {
        out.push_str("margin-inline-end:");
        write_compact_px(canonical_f32(v), out);
        out.push(';');
    }
    if let Some(tf) = patch.transform {
        tf.write_css_decls_into(true, out);
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
        write_negated_media_css_conditions(&query, &mut negated);
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
        if !write_media_css_condition(&rule.query, &mut condition) {
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

