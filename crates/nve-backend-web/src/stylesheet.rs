//! Option 5 Hybrid CSS stylesheet cache: compact declaration emitters, structural vs volatile
//! lane resolution, bounded `MAX_DYNAMIC_RULES`, and batched `wasm32` `CSSStyleSheet` flushing.

use nve_abi::hash::{HashMap, HashSet};

use nve_abi::{
    ColorRgba, ComponentKind, Dimension, EventMask, FlexAlign, FlexDirection, FlexJustify,
    LayoutProps, StyleProps,
};

use crate::css::{
    canonical_f32, write_compact_color, write_compact_dim, write_compact_edges, write_compact_num,
    write_compact_px, MirrorElement, RawStyleProps,
};
use crate::style_hash::{
    format_structural_class, hash_structural_style, is_inheriting_leaf_text, is_volatile_dimension,
    is_volatile_opacity,
};

#[cfg(target_arch = "wasm32")]
use crate::css::NVE_BASE_RESET_CSS;

/// Hard upper bound on dynamic structural CSS rules in [`StyleSheetCache`].
pub const MAX_DYNAMIC_RULES: usize = 4096;

/// Incremental rule batch size threshold above which `wasm32` allocates a new chunked
/// `CSSStyleSheet` via `replace_sync` instead of per-rule `insert_rule`.
pub const INCREMENTAL_RULE_BATCH_THRESHOLD: usize = 16;

/// Emits compact non-default layout declarations into `out`, optionally skipping volatile dims.
pub fn write_compact_layout_into(l: &LayoutProps, skip_volatile: bool, out: &mut String) {
    match l.flex_direction {
        FlexDirection::Column => {}
        FlexDirection::Row => out.push_str("flex-direction:row;"),
        FlexDirection::ColumnReverse => out.push_str("flex-direction:column-reverse;"),
        FlexDirection::RowReverse => out.push_str("flex-direction:row-reverse;"),
    }
    match l.justify_content {
        FlexJustify::FlexStart => {}
        FlexJustify::Center => out.push_str("justify-content:center;"),
        FlexJustify::FlexEnd => out.push_str("justify-content:flex-end;"),
        FlexJustify::SpaceBetween => out.push_str("justify-content:space-between;"),
        FlexJustify::SpaceAround => out.push_str("justify-content:space-around;"),
        FlexJustify::SpaceEvenly => out.push_str("justify-content:space-evenly;"),
    }
    match l.align_items {
        FlexAlign::Stretch => {}
        FlexAlign::FlexStart => out.push_str("align-items:flex-start;"),
        FlexAlign::Center => out.push_str("align-items:center;"),
        FlexAlign::FlexEnd => out.push_str("align-items:flex-end;"),
        FlexAlign::Baseline => out.push_str("align-items:baseline;"),
    }
    let fg = canonical_f32(l.flex_grow);
    if fg != 0.0 {
        out.push_str("flex-grow:");
        write_compact_num(fg, out);
        out.push(';');
    }
    let fs = canonical_f32(l.flex_shrink);
    if (fs - 1.0).abs() > f32::EPSILON {
        out.push_str("flex-shrink:");
        write_compact_num(fs, out);
        out.push(';');
    }
    let gap = canonical_f32(l.gap);
    if gap != 0.0 {
        out.push_str("gap:");
        write_compact_px(gap, out);
        out.push(';');
    }

    for (prop, dim) in [
        ("width:", l.width),
        ("height:", l.height),
        ("min-width:", l.min_width),
        ("min-height:", l.min_height),
        ("max-width:", l.max_width),
        ("max-height:", l.max_height),
    ] {
        if dim != Dimension::Auto && !(skip_volatile && is_volatile_dimension(dim)) {
            out.push_str(prop);
            write_compact_dim(dim, out);
            out.push(';');
        }
    }

    write_compact_edges("padding", l.padding, out);
    write_compact_edges("margin", l.margin, out);
}

/// Emits compact non-default visual & raw style declarations into `out`.
pub fn write_compact_style_into(
    s: &StyleProps,
    raw: &RawStyleProps,
    leaf_text: bool,
    skip_volatile_opacity: bool,
    out: &mut String,
) {
    if s.background.a > 0 {
        out.push_str("background-color:");
        write_compact_color(s.background, out);
        out.push(';');
    }
    if !leaf_text && s.text_color != ColorRgba::BLACK {
        out.push_str("color:");
        write_compact_color(s.text_color, out);
        out.push(';');
    }
    let bw = canonical_f32(s.border_width);
    if bw > 0.0 {
        out.push_str("border:");
        write_compact_px(bw, out);
        out.push_str(" solid ");
        write_compact_color(s.border_color, out);
        out.push(';');
    }
    let br = canonical_f32(s.border_radius);
    if br != 0.0 {
        out.push_str("border-radius:");
        write_compact_px(br, out);
        out.push(';');
    }
    let op = canonical_f32(s.opacity);
    if (op - 1.0).abs() > f32::EPSILON && !(skip_volatile_opacity && is_volatile_opacity(op)) {
        out.push_str("opacity:");
        write_compact_num(op, out);
        out.push(';');
    }
    let fs = canonical_f32(s.font_size);
    if !leaf_text && (fs - 16.0).abs() > f32::EPSILON {
        out.push_str("font-size:");
        write_compact_px(fs, out);
        out.push(';');
    }
    if s.z_index != 0 {
        out.push_str("z-index:");
        nve_abi::num::write_i64(out, i64::from(s.z_index));
        out.push(';');
    }
    raw.write_compact_into(leaf_text, out);
}

/// Emits ONLY volatile dimensions, non-discrete opacity, and Tier-1 `patch_css` into `out`.
pub fn write_volatile_inline_into(
    l: &LayoutProps,
    s: &StyleProps,
    patch_css: &str,
    out: &mut String,
) {
    for (prop, dim) in [
        ("width:", l.width),
        ("height:", l.height),
        ("min-width:", l.min_width),
        ("min-height:", l.min_height),
        ("max-width:", l.max_width),
        ("max-height:", l.max_height),
    ] {
        if is_volatile_dimension(dim) {
            out.push_str(prop);
            write_compact_dim(dim, out);
            out.push(';');
        }
    }
    if is_volatile_opacity(s.opacity) && (canonical_f32(s.opacity) - 1.0).abs() > f32::EPSILON {
        out.push_str("opacity:");
        write_compact_num(s.opacity, out);
        out.push(';');
    }
    if !patch_css.is_empty() {
        out.push_str(patch_css);
    }
}

/// Resolved Option 5 Hybrid presentation for a single DOM element.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResolvedNodeStyle {
    pub class_name: String,
    pub inline_style: String,
    pub structural_rule: Option<(String, String)>,
}

/// Computes the deterministic class list, optional structural rule, and optional inline lane
/// declarations for `node`.
pub fn resolve_node_style(
    node: &MirrorElement,
    parent: Option<&MirrorElement>,
    allow_new_rule: bool,
    existing_rule_check: impl Fn(&str) -> bool,
) -> ResolvedNodeStyle {
    let leaf_text = is_inheriting_leaf_text(parent, node);
    let mut class_name = String::with_capacity(28);
    class_name.push_str("nve");

    match node.kind {
        ComponentKind::ScrollView => class_name.push_str(" nve-s"),
        ComponentKind::Pressable => class_name.push_str(" nve-b"),
        ComponentKind::TextInput => class_name.push_str(" nve-i"),
        _ if (node.event_mask & EventMask::PRESS) != 0 => class_name.push_str(" nve-p"),
        _ => {}
    }
    if leaf_text {
        class_name.push_str(" nve-tl");
    }

    let mut inline_style = String::new();

    if node.is_consecutive_frame_volatile() {
        write_compact_layout_into(&node.layout, false, &mut inline_style);
        write_compact_style_into(&node.style, &node.raw_style, leaf_text, false, &mut inline_style);
        if !node.patch_css.is_empty() {
            inline_style.push_str(&node.patch_css);
        }
        return ResolvedNodeStyle {
            class_name,
            inline_style,
            structural_rule: None,
        };
    }

    let mut structural_decls = String::with_capacity(96);
    write_compact_layout_into(&node.layout, true, &mut structural_decls);
    write_compact_style_into(
        &node.style,
        &node.raw_style,
        leaf_text,
        true,
        &mut structural_decls,
    );

    let structural_rule = if structural_decls.is_empty() {
        None
    } else {
        let hash = hash_structural_style(&node.layout, &node.style, &node.raw_style, leaf_text);
        let cls = format_structural_class(hash);
        if existing_rule_check(&cls) || allow_new_rule {
            class_name.push(' ');
            class_name.push_str(&cls);
            Some((cls, structural_decls))
        } else {
            inline_style.push_str(&structural_decls);
            None
        }
    };

    write_volatile_inline_into(&node.layout, &node.style, &node.patch_css, &mut inline_style);

    ResolvedNodeStyle {
        class_name,
        inline_style,
        structural_rule,
    }
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(inline_js = "
export function nve_adopt_stylesheet(doc, sheet) {
  try {
    if (!doc || !('adoptedStyleSheets' in doc)) return false;
    doc.adoptedStyleSheets = [...doc.adoptedStyleSheets, sheet];
    return true;
  } catch (_) {
    return false;
  }
}
")]
extern "C" {
    fn nve_adopt_stylesheet(doc: &web_sys::Document, sheet: &web_sys::CssStyleSheet) -> bool;
}

/// Appends one `.{class}{{decls}}` rule (no `core::fmt` on the wasm hot path).
#[cfg(target_arch = "wasm32")]
fn push_rule(out: &mut String, class_name: &str, decls: &str) {
    out.push('.');
    out.push_str(class_name);
    out.push('{');
    out.push_str(decls);
    out.push('}');
}

/// Bounded content-addressed structural CSS cache with batched `wasm32` stylesheet flushing.
#[derive(Default)]
pub struct StyleSheetCache {
    rules: HashMap<String, String>,
    inserted: HashSet<String>,
    pending: Vec<(String, String)>,
    base_mounted: bool,
    #[cfg(target_arch = "wasm32")]
    initial_flushed: bool,
    #[cfg(target_arch = "wasm32")]
    use_adopted: Option<bool>,
    #[cfg(target_arch = "wasm32")]
    base_sheet: Option<web_sys::CssStyleSheet>,
    #[cfg(target_arch = "wasm32")]
    dynamic_sheets: Vec<web_sys::CssStyleSheet>,
    #[cfg(target_arch = "wasm32")]
    fallback_style_el: Option<web_sys::Element>,
}

impl StyleSheetCache {
    pub fn new() -> Self {
        Self::default()
    }

    #[inline]
    pub fn rule_count(&self) -> usize {
        self.rules.len().max(self.inserted.len())
    }

    #[inline]
    pub fn contains_rule(&self, class_name: &str) -> bool {
        self.inserted.contains(class_name) || self.rules.contains_key(class_name)
    }

    #[inline]
    pub fn can_insert_new(&self) -> bool {
        self.rule_count() < MAX_DYNAMIC_RULES
    }

    #[inline]
    pub fn mark_base_mounted(&mut self) {
        self.base_mounted = true;
    }

    pub fn seed_ssr_hashes(&mut self, csv: &str) -> usize {
        let mut count = 0usize;
        for token in csv.split(',') {
            let trimmed = token.trim();
            if !trimmed.is_empty() && self.inserted.insert(trimmed.to_owned()) {
                count += 1;
            }
        }
        count
    }

    pub fn ensure_rule(&mut self, class_name: String, decls: String) -> bool {
        if self.inserted.contains(&class_name) || self.rules.contains_key(&class_name) {
            return true;
        }
        if !self.can_insert_new() {
            return false;
        }
        self.rules.insert(class_name.clone(), decls.clone());
        self.pending.push((class_name, decls));
        true
    }

    #[inline]
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    #[inline]
    pub fn rules(&self) -> &HashMap<String, String> {
        &self.rules
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn flush_to_dom(&mut self) {
        self.base_mounted = true;
        for (cls, _) in self.pending.drain(..) {
            self.inserted.insert(cls);
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub fn flush_to_dom(&mut self) {
        let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
            for (cls, _) in self.pending.drain(..) {
                self.inserted.insert(cls);
            }
            return;
        };

        if !self.base_mounted {
            self.mount_base_reset(&doc);
            self.base_mounted = true;
        }

        if self.pending.is_empty() {
            return;
        }

        let batch = std::mem::take(&mut self.pending);
        for (cls, _) in &batch {
            self.inserted.insert(cls.clone());
        }

        let use_adopted = self.use_adopted.unwrap_or(true);
        if use_adopted {
            if !self.initial_flushed
                || batch.len() > INCREMENTAL_RULE_BATCH_THRESHOLD
                || self.dynamic_sheets.is_empty()
            {
                let mut text = String::with_capacity(batch.len() * 64);
                for (cls, decls) in &batch {
                    push_rule(&mut text, cls, decls);
                    text.push('\n');
                }
                if let Ok(sheet) = web_sys::CssStyleSheet::new()
                    && sheet.replace_sync(&text).is_ok()
                    && nve_adopt_stylesheet(&doc, &sheet)
                {
                    self.dynamic_sheets.push(sheet);
                    self.initial_flushed = true;
                    self.use_adopted = Some(true);
                    return;
                }
                self.use_adopted = Some(false);
            } else if let Some(sheet) = self.dynamic_sheets.last() {
                let mut all_ok = true;
                let mut rule = String::with_capacity(96);
                for (cls, decls) in &batch {
                    rule.clear();
                    push_rule(&mut rule, cls, decls);
                    if sheet.insert_rule(&rule).is_err() {
                        all_ok = false;
                        break;
                    }
                }
                if all_ok {
                    return;
                }
            }
        }

        self.append_to_fallback_style(&doc, &batch);
        self.initial_flushed = true;
    }

    #[cfg(target_arch = "wasm32")]
    fn style_parent(doc: &web_sys::Document) -> Option<web_sys::Element> {
        doc.query_selector("head")
            .ok()
            .flatten()
            .or_else(|| doc.document_element())
    }

    #[cfg(target_arch = "wasm32")]
    fn mount_base_reset(&mut self, doc: &web_sys::Document) {
        if let Ok(sheet) = web_sys::CssStyleSheet::new()
            && sheet.replace_sync(NVE_BASE_RESET_CSS).is_ok()
            && nve_adopt_stylesheet(doc, &sheet)
        {
            self.base_sheet = Some(sheet);
            self.use_adopted = Some(true);
            return;
        }
        self.use_adopted = Some(false);
        if doc.get_element_by_id("nve-base-styles").is_none()
            && let Ok(style_el) = doc.create_element("style")
        {
            style_el.set_id("nve-base-styles");
            let _ = style_el.set_attribute("data-nve-base", "");
            style_el.set_text_content(Some(NVE_BASE_RESET_CSS));
            if let Some(parent) = Self::style_parent(doc) {
                let _ = parent.append_child(&style_el);
            }
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn append_to_fallback_style(&mut self, doc: &web_sys::Document, batch: &[(String, String)]) {
        let style_el = match self.fallback_style_el.clone() {
            Some(el) => el,
            None => {
                let existing = doc.get_element_by_id("nve-styles");
                let el = match existing {
                    Some(el) => el,
                    None => {
                        let Ok(created) = doc.create_element("style") else {
                            return;
                        };
                        created.set_id("nve-styles");
                        if let Some(parent) = Self::style_parent(doc) {
                            let _ = parent.append_child(&created);
                        }
                        created
                    }
                };
                self.fallback_style_el = Some(el.clone());
                el
            }
        };
        let mut css = style_el.text_content().unwrap_or_default();
        for (cls, decls) in batch {
            push_rule(&mut css, cls, decls);
            css.push('\n');
        }
        style_el.set_text_content(Some(&css));
    }
}
