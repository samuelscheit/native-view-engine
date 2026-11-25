//! Option 5 Hybrid CSS stylesheet cache: compact declaration emitters, structural vs volatile
//! lane resolution, bounded `MAX_DYNAMIC_RULES`, and batched `wasm32` `CSSStyleSheet` flushing.

use nve_abi::hash::{HashMap, HashSet};
use nve_abi::{ComponentKind, EventMask};

pub use crate::css::{
    write_compact_layout_into, write_compact_style_into, write_volatile_inline_into,
};
use crate::css::MirrorElement;
use crate::media_css::{compile_node_media_rules, CompiledMediaRule};
use crate::style_hash::{format_structural_class, hash_structural_style, is_inheriting_leaf_text};

#[cfg(target_arch = "wasm32")]
use crate::css::NVE_BASE_RESET_CSS;

/// Hard upper bound on dynamic structural CSS rules in [`StyleSheetCache`].
pub const MAX_DYNAMIC_RULES: usize = 4096;

/// Incremental rule batch size threshold above which `wasm32` allocates a new chunked
/// `CSSStyleSheet` via `replace_sync` instead of per-rule `insert_rule`.
pub const INCREMENTAL_RULE_BATCH_THRESHOLD: usize = 16;

/// Resolved Option 5 Hybrid presentation for a single DOM element.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResolvedNodeStyle {
    pub class_name: String,
    pub inline_style: String,
    pub structural_rule: Option<(String, String)>,
    pub media_rules: Vec<CompiledMediaRule>,
}

/// Computes the deterministic class list, optional structural rule, responsive `@media` rules,
/// and optional inline lane declarations for `node`.
pub fn resolve_node_style(
    node: &MirrorElement,
    parent: Option<&MirrorElement>,
    allow_new_rule: bool,
    existing_rule_check: impl Fn(&str) -> bool,
) -> ResolvedNodeStyle {
    let leaf_text = is_inheriting_leaf_text(parent, node);
    let mut class_name = String::with_capacity(36);
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

    let mut media_rules = Vec::new();
    if node.show_on.is_some() || !node.responsive_rules.is_empty() {
        compile_node_media_rules(node.show_on, &node.responsive_rules, &mut media_rules);
    }

    let mut inline_style = String::new();

    if node.is_consecutive_frame_volatile() {
        write_compact_layout_into(&node.layout, false, &mut inline_style);
        write_compact_style_into(&node.style, &node.raw_style, leaf_text, false, &mut inline_style);
        if !node.patch_css.is_empty() {
            inline_style.push_str(&node.patch_css);
        }
        for rule in &media_rules {
            class_name.push(' ');
            class_name.push_str(&rule.class_name);
        }
        return ResolvedNodeStyle {
            class_name,
            inline_style,
            structural_rule: None,
            media_rules,
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

    for rule in &media_rules {
        class_name.push(' ');
        class_name.push_str(&rule.class_name);
    }

    write_volatile_inline_into(&node.layout, &node.style, &node.patch_css, &mut inline_style);

    ResolvedNodeStyle {
        class_name,
        inline_style,
        structural_rule,
        media_rules,
    }
}

/// One cached stylesheet rule stored in [`StyleSheetCache`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CachedRule {
    Structural(String),
    Media { condition: String, decls: String },
}

impl CachedRule {
    /// Writes this rule's complete CSS text into `out`.
    pub fn write_css_into(&self, class_name: &str, out: &mut String) {
        match self {
            Self::Structural(decls) => {
                out.push('.');
                out.push_str(class_name);
                out.push('{');
                out.push_str(decls);
                out.push('}');
            }
            Self::Media { condition, decls } => {
                out.push_str("@media ");
                out.push_str(condition);
                out.push_str("{.");
                out.push_str(class_name);
                out.push('{');
                out.push_str(decls);
                out.push_str("}}");
            }
        }
    }
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(inline_js = "
export function nve_insert_adopted_stylesheet(doc, sheet, before_sheet) {
  try {
    if (!doc || !('adoptedStyleSheets' in doc)) return false;
    const cur = Array.from(doc.adoptedStyleSheets);
    const idx = before_sheet ? cur.indexOf(before_sheet) : -1;
    if (idx >= 0) {
      cur.splice(idx, 0, sheet);
    } else {
      cur.push(sheet);
    }
    doc.adoptedStyleSheets = cur;
    return true;
  } catch (_) {
    return false;
  }
}
")]
extern "C" {
    fn nve_insert_adopted_stylesheet(
        doc: &web_sys::Document,
        sheet: &web_sys::CssStyleSheet,
        before_sheet: Option<&web_sys::CssStyleSheet>,
    ) -> bool;
}

/// Bounded content-addressed structural & `@media` CSS cache with cascade-ordered `wasm32` flushing.
#[derive(Default)]
pub struct StyleSheetCache {
    rules: HashMap<String, String>,
    media_rules: HashMap<String, CompiledMediaRule>,
    inserted: HashSet<String>,
    pending: Vec<(String, CachedRule)>,
    base_mounted: bool,
    #[cfg(not(target_arch = "wasm32"))]
    flushed_css_rules: Vec<String>,
    #[cfg(target_arch = "wasm32")]
    initial_flushed: bool,
    #[cfg(target_arch = "wasm32")]
    use_adopted: Option<bool>,
    #[cfg(target_arch = "wasm32")]
    base_sheet: Option<web_sys::CssStyleSheet>,
    #[cfg(target_arch = "wasm32")]
    dynamic_sheets: Vec<web_sys::CssStyleSheet>,
    #[cfg(target_arch = "wasm32")]
    dynamic_sheet_last_len: u32,
    #[cfg(target_arch = "wasm32")]
    media_sheets: Vec<web_sys::CssStyleSheet>,
    #[cfg(target_arch = "wasm32")]
    media_sheet_last_len: u32,
    #[cfg(target_arch = "wasm32")]
    fallback_style_el: Option<web_sys::Element>,
    #[cfg(target_arch = "wasm32")]
    fallback_media_style_el: Option<web_sys::Element>,
}

impl StyleSheetCache {
    pub fn new() -> Self {
        Self::default()
    }

    #[inline]
    pub fn rule_count(&self) -> usize {
        self.inserted.len().saturating_add(self.pending.len())
    }

    #[inline]
    pub fn contains_rule(&self, class_name: &str) -> bool {
        self.inserted.contains(class_name)
            || self.rules.contains_key(class_name)
            || self.media_rules.contains_key(class_name)
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
            if !trimmed.is_empty()
                && !self.contains_rule(trimmed)
                && self.inserted.insert(trimmed.to_owned())
            {
                count += 1;
            }
        }
        count
    }

    pub fn ensure_rule(&mut self, class_name: String, decls: String) -> bool {
        if self.contains_rule(&class_name) {
            return true;
        }
        if !self.can_insert_new() {
            return false;
        }
        self.rules.insert(class_name.clone(), decls.clone());
        self.pending.push((class_name, CachedRule::Structural(decls)));
        true
    }

    /// Queues a content-addressed `@media` rule (`nve-m-XXXXXXXX`) unless already cached or seeded.
    pub fn ensure_media_rule(&mut self, rule: CompiledMediaRule) -> bool {
        if self.contains_rule(&rule.class_name) {
            return true;
        }
        if !self.can_insert_new() {
            return false;
        }
        let key = rule.class_name.clone();
        let cached = CachedRule::Media {
            condition: rule.condition.clone(),
            decls: rule.decls.clone(),
        };
        self.media_rules.insert(key.clone(), rule);
        self.pending.push((key, cached));
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

    #[inline]
    pub fn media_rules(&self) -> &HashMap<String, CompiledMediaRule> {
        &self.media_rules
    }

    /// Returns all CSS rules flushed so far in exact cascade order (structural first, `@media`
    /// second) on native targets for deterministic testing.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn flushed_css_rules(&self) -> &[String] {
        &self.flushed_css_rules
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn flush_to_dom(&mut self) {
        self.base_mounted = true;
        if self.pending.is_empty() {
            return;
        }
        let batch = std::mem::take(&mut self.pending);
        let mut structural = Vec::new();
        let mut media = Vec::new();
        for (cls, rule) in batch {
            self.inserted.insert(cls.clone());
            match rule {
                CachedRule::Structural(_) => structural.push((cls, rule)),
                CachedRule::Media { .. } => media.push((cls, rule)),
            }
        }
        // Emit structural rules strictly before `@media` rules so cascade order is preserved.
        for (cls, rule) in structural.into_iter().chain(media) {
            let mut css = String::with_capacity(64);
            rule.write_css_into(&cls, &mut css);
            self.flushed_css_rules.push(css);
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
        let mut structural_batch = Vec::new();
        let mut media_batch = Vec::new();
        for (cls, rule) in batch {
            self.inserted.insert(cls.clone());
            match rule {
                CachedRule::Structural(_) => structural_batch.push((cls, rule)),
                CachedRule::Media { .. } => media_batch.push((cls, rule)),
            }
        }

        // Flush ALL structural rules BEFORE ALL `@media` rules into separate cascade layers.
        if !structural_batch.is_empty() {
            self.flush_layer(&doc, &structural_batch, false);
        }
        if !media_batch.is_empty() {
            self.flush_layer(&doc, &media_batch, true);
        }
        self.initial_flushed = true;
    }

    #[cfg(target_arch = "wasm32")]
    fn flush_layer(&mut self, doc: &web_sys::Document, batch: &[(String, CachedRule)], is_media: bool) {
        let use_adopted = self.use_adopted.unwrap_or(true);
        if use_adopted {
            let sheets_empty = if is_media {
                self.media_sheets.is_empty()
            } else {
                self.dynamic_sheets.is_empty()
            };
            if !self.initial_flushed
                || batch.len() > INCREMENTAL_RULE_BATCH_THRESHOLD
                || sheets_empty
            {
                let mut text = String::with_capacity(batch.len() * 64);
                for (cls, rule) in batch {
                    rule.write_css_into(cls, &mut text);
                    text.push('\n');
                }
                let before = if is_media { None } else { self.media_sheets.first() };
                if let Ok(sheet) = web_sys::CssStyleSheet::new()
                    && sheet.replace_sync(&text).is_ok()
                    && nve_insert_adopted_stylesheet(doc, &sheet, before)
                {
                    if is_media {
                        self.media_sheets.push(sheet);
                        self.media_sheet_last_len = batch.len() as u32;
                    } else {
                        self.dynamic_sheets.push(sheet);
                        self.dynamic_sheet_last_len = batch.len() as u32;
                    }
                    self.use_adopted = Some(true);
                    return;
                }
                self.use_adopted = Some(false);
            } else {
                let (sheet_opt, last_len) = if is_media {
                    (self.media_sheets.last(), &mut self.media_sheet_last_len)
                } else {
                    (self.dynamic_sheets.last(), &mut self.dynamic_sheet_last_len)
                };
                if let Some(sheet) = sheet_opt {
                    let mut all_ok = true;
                    let mut rule_buf = String::with_capacity(96);
                    for (cls, rule) in batch {
                        rule_buf.clear();
                        rule.write_css_into(cls, &mut rule_buf);
                        if sheet.insert_rule_with_index(&rule_buf, *last_len).is_err() {
                            all_ok = false;
                            break;
                        }
                        *last_len = last_len.saturating_add(1);
                    }
                    if all_ok {
                        return;
                    }
                }
            }
        }

        self.append_to_fallback_style(doc, batch, is_media);
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
        let before = self.dynamic_sheets.first().or_else(|| self.media_sheets.first());
        if let Ok(sheet) = web_sys::CssStyleSheet::new()
            && sheet.replace_sync(NVE_BASE_RESET_CSS).is_ok()
            && nve_insert_adopted_stylesheet(doc, &sheet, before)
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
    fn append_to_fallback_style(
        &mut self,
        doc: &web_sys::Document,
        batch: &[(String, CachedRule)],
        is_media: bool,
    ) {
        let id = if is_media { "nve-media-styles" } else { "nve-styles" };
        let existing_cached = if is_media {
            self.fallback_media_style_el.clone()
        } else {
            self.fallback_style_el.clone()
        };
        let style_el = match existing_cached {
            Some(el) => el,
            None => {
                let el = match doc.get_element_by_id(id) {
                    Some(el) => el,
                    None => {
                        let Ok(created) = doc.create_element("style") else {
                            return;
                        };
                        created.set_id(id);
                        if let Some(parent) = Self::style_parent(doc) {
                            if !is_media
                                && let Some(media_el) = self.fallback_media_style_el.as_ref()
                            {
                                let _ = parent.insert_before(&created, Some(media_el));
                            } else {
                                let _ = parent.append_child(&created);
                            }
                        }
                        created
                    }
                };
                if is_media {
                    self.fallback_media_style_el = Some(el.clone());
                } else {
                    self.fallback_style_el = Some(el.clone());
                }
                el
            }
        };
        let mut css = style_el.text_content().unwrap_or_default();
        for (cls, rule) in batch {
            rule.write_css_into(cls, &mut css);
            css.push('\n');
        }
        style_el.set_text_content(Some(&css));
    }
}
