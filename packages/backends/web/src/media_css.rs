//! Zero-serialization `@media` CSS compilation for [`MediaQuery`] visibility (`show_on`) and
//! table-generic [`RowPatch`] / [`RowPatchView`] responsive style overrides.

use std::fmt::Write as _;

use nve_abi::ColorScheme;
use nve_components::media::{MediaPointer, MediaQuery};
use nve_components::tables::ResponsiveRulesView;
use nve_store::NodeStore;

use crate::css::format_content_class;
pub use crate::patch_css::write_row_patch_view_decls;

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
        let mut out = String::with_capacity(
            self.condition.len() + self.class_name.len() + self.decls.len() + 12,
        );
        self.write_css_rule_into(&mut out);
        out
    }
}

/// Formats a deterministic content-addressed class token `nve-m-<8hex>` for `(condition, decls)`.
pub fn format_media_class(condition: &str, decls: &str) -> String {
    format_content_class("nve-m-", &[condition.as_bytes(), &[0], decls.as_bytes()])
}

fn push_unique_media(out: &mut Vec<CompiledMediaRule>, rule: CompiledMediaRule) {
    if !out.iter().any(|existing| existing.class_name == rule.class_name) {
        out.push(rule);
    }
}

fn push_show_on_rules(show_on: Option<MediaQuery>, out: &mut Vec<CompiledMediaRule>) {
    if let Some(query) = show_on {
        let mut negated = Vec::new();
        write_negated_media_css_conditions(&query, &mut negated);
        const HIDE_DECL: &str = "display:none!important;";
        for condition in negated {
            let class_name = format_media_class(&condition, HIDE_DECL);
            push_unique_media(
                out,
                CompiledMediaRule {
                    class_name,
                    condition,
                    decls: HIDE_DECL.to_owned(),
                },
            );
        }
    }
}

/// Compiles `show_on` + a zero-copy [`ResponsiveRulesView`] into content-addressed CSS rules.
pub fn compile_node_media_rules_view(
    show_on: Option<MediaQuery>,
    rules: ResponsiveRulesView<'_>,
    store: Option<&NodeStore>,
    out: &mut Vec<CompiledMediaRule>,
) {
    push_show_on_rules(show_on, out);
    for (query, patch) in rules.iter() {
        if patch.is_empty() || !query.is_valid() {
            continue;
        }
        let mut condition = String::new();
        if !write_media_css_condition(&query, &mut condition) {
            continue;
        }
        let mut decls = String::new();
        write_row_patch_view_decls(patch, store, &mut decls);
        if decls.is_empty() {
            continue;
        }
        let class_name = format_media_class(&condition, &decls);
        push_unique_media(
            out,
            CompiledMediaRule {
                class_name,
                condition,
                decls,
            },
        );
    }
}

