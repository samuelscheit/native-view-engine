//! `wasm32` adopted-stylesheet & fallback `<style>` flushing for [`crate::StyleSheetCache`].

use crate::css::NVE_BASE_RESET_CSS;
use crate::stylesheet::{
    CachedRule, StyleSheetCache, INCREMENTAL_RULE_BATCH_THRESHOLD, STATE_TIER_COUNT,
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum CascadeLayer {
    Structural,
    Media,
    State(usize),
}

const FALLBACK_STATE_IDS: [&str; STATE_TIER_COUNT] = [
    "nve-state-styles-0",
    "nve-state-styles-1",
    "nve-state-styles-2",
    "nve-state-styles-3",
    "nve-state-styles-4",
];

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

impl StyleSheetCache {
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
        let mut state_tiers: [Vec<(String, CachedRule)>; STATE_TIER_COUNT] = Default::default();
        for (cls, rule) in batch {
            self.inserted.insert(cls.clone());
            match rule {
                CachedRule::Structural(_) => structural_batch.push((cls, rule)),
                CachedRule::Media { .. } => media_batch.push((cls, rule)),
                CachedRule::State(sr) => {
                    if let Some(tier) = sr.state.cascade_tier() {
                        state_tiers[tier].push((cls, CachedRule::State(sr)));
                    }
                }
            }
        }

        if !structural_batch.is_empty() {
            self.flush_layer(&doc, &structural_batch, CascadeLayer::Structural);
        }
        if !media_batch.is_empty() {
            self.flush_layer(&doc, &media_batch, CascadeLayer::Media);
        }
        for (tier, mut tier_batch) in state_tiers.into_iter().enumerate() {
            if !tier_batch.is_empty() {
                tier_batch.sort_by_key(|(_, rule)| match rule {
                    CachedRule::State(sr) => sr.state.cascade_order(),
                    _ => 0,
                });
                self.flush_layer(&doc, &tier_batch, CascadeLayer::State(tier));
            }
        }
        self.initial_flushed = true;
    }

    fn first_adopted_sheet_after(&self, layer: CascadeLayer) -> Option<&web_sys::CssStyleSheet> {
        match layer {
            CascadeLayer::Structural => self
                .media_sheets
                .first()
                .or_else(|| self.state_sheets.iter().find_map(|s| s.first())),
            CascadeLayer::Media => self.state_sheets.iter().find_map(|s| s.first()),
            CascadeLayer::State(tier) => self.state_sheets[(tier + 1)..]
                .iter()
                .find_map(|s| s.first()),
        }
    }

    fn first_fallback_el_after(&self, layer: CascadeLayer) -> Option<&web_sys::Element> {
        match layer {
            CascadeLayer::Structural => self
                .fallback_media_style_el
                .as_ref()
                .or_else(|| self.fallback_state_style_els.iter().find_map(|e| e.as_ref())),
            CascadeLayer::Media => self.fallback_state_style_els.iter().find_map(|e| e.as_ref()),
            CascadeLayer::State(tier) => self.fallback_state_style_els[(tier + 1)..]
                .iter()
                .find_map(|e| e.as_ref()),
        }
    }

    fn layer_sheets_mut(
        &mut self,
        layer: CascadeLayer,
    ) -> (&mut Vec<web_sys::CssStyleSheet>, &mut u32) {
        match layer {
            CascadeLayer::Structural => (&mut self.dynamic_sheets, &mut self.dynamic_sheet_last_len),
            CascadeLayer::Media => (&mut self.media_sheets, &mut self.media_sheet_last_len),
            CascadeLayer::State(tier) => (
                &mut self.state_sheets[tier],
                &mut self.state_sheet_last_lens[tier],
            ),
        }
    }

    fn flush_layer(
        &mut self,
        doc: &web_sys::Document,
        batch: &[(String, CachedRule)],
        layer: CascadeLayer,
    ) {
        let use_adopted = self.use_adopted.unwrap_or(true);
        if use_adopted {
            let sheets_empty = match layer {
                CascadeLayer::Structural => self.dynamic_sheets.is_empty(),
                CascadeLayer::Media => self.media_sheets.is_empty(),
                CascadeLayer::State(tier) => self.state_sheets[tier].is_empty(),
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
                let before = self.first_adopted_sheet_after(layer).cloned();
                if let Ok(sheet) = web_sys::CssStyleSheet::new()
                    && sheet.replace_sync(&text).is_ok()
                    && nve_insert_adopted_stylesheet(doc, &sheet, before.as_ref())
                {
                    let (sheets, last_len) = self.layer_sheets_mut(layer);
                    sheets.push(sheet);
                    *last_len = batch.len() as u32;
                    self.use_adopted = Some(true);
                    return;
                }
                self.use_adopted = Some(false);
            } else {
                let (sheets, last_len) = self.layer_sheets_mut(layer);
                if let Some(sheet) = sheets.last() {
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

        self.append_to_fallback_style(doc, batch, layer);
    }

    /// Creates a runtime `<style>` element carrying the SSR page's CSP nonce (read from the
    /// server-emitted `<style data-nve-base nonce>`), so strict `style-src 'nonce-…'` policies
    /// accept fallback stylesheets. Uses the `nonce` IDL property: browsers hide the attribute
    /// value after parsing.
    fn create_style_el(doc: &web_sys::Document) -> Option<web_sys::Element> {
        use wasm_bindgen::JsCast as _;
        let el = doc.create_element("style").ok()?;
        let nonce = doc
            .query_selector("style[data-nve-base][nonce]")
            .ok()
            .flatten()
            .and_then(|s| s.dyn_into::<web_sys::HtmlElement>().ok())
            .map(|s| s.nonce())
            .filter(|n| !n.is_empty());
        if let Some(nonce) = nonce {
            let _ = el.set_attribute("nonce", &nonce);
        }
        Some(el)
    }

    fn style_parent(doc: &web_sys::Document) -> Option<web_sys::Element> {
        doc.query_selector("head")
            .ok()
            .flatten()
            .or_else(|| doc.document_element())
    }

    fn mount_base_reset(&mut self, doc: &web_sys::Document) {
        let before = self
            .dynamic_sheets
            .first()
            .or_else(|| self.first_adopted_sheet_after(CascadeLayer::Structural));
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
            && let Some(style_el) = Self::create_style_el(doc)
        {
            style_el.set_id("nve-base-styles");
            let _ = style_el.set_attribute("data-nve-base", "");
            style_el.set_text_content(Some(NVE_BASE_RESET_CSS));
            if let Some(parent) = Self::style_parent(doc) {
                let _ = parent.append_child(&style_el);
            }
        }
    }

    fn append_to_fallback_style(
        &mut self,
        doc: &web_sys::Document,
        batch: &[(String, CachedRule)],
        layer: CascadeLayer,
    ) {
        let id = match layer {
            CascadeLayer::Structural => "nve-styles",
            CascadeLayer::Media => "nve-media-styles",
            CascadeLayer::State(tier) => FALLBACK_STATE_IDS[tier],
        };
        let existing_cached = match layer {
            CascadeLayer::Structural => self.fallback_style_el.clone(),
            CascadeLayer::Media => self.fallback_media_style_el.clone(),
            CascadeLayer::State(tier) => self.fallback_state_style_els[tier].clone(),
        };
        let style_el = match existing_cached {
            Some(el) => el,
            None => {
                let el = match doc.get_element_by_id(id) {
                    Some(el) => el,
                    None => {
                        let Some(created) = Self::create_style_el(doc) else {
                            return;
                        };
                        created.set_id(id);
                        if let Some(parent) = Self::style_parent(doc) {
                            if let Some(before) = self.first_fallback_el_after(layer) {
                                let _ = parent.insert_before(&created, Some(before));
                            } else {
                                let _ = parent.append_child(&created);
                            }
                        }
                        created
                    }
                };
                match layer {
                    CascadeLayer::Structural => self.fallback_style_el = Some(el.clone()),
                    CascadeLayer::Media => self.fallback_media_style_el = Some(el.clone()),
                    CascadeLayer::State(tier) => {
                        self.fallback_state_style_els[tier] = Some(el.clone());
                    }
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
