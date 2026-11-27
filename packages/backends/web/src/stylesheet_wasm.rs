//! Live `wasm32` CSS stylesheet mounting & flushing via Constructable Stylesheets
//! (`document.adoptedStyleSheets`, with `<style>` element fallback).
//!
//! Preserves exact CSS cascade order:
//! `base reset -> structural (.nve-c-*) -> @media (.nve-m-*) -> state tiers (Hover < Focus < FocusVisible < Active < Disabled)`.

use crate::css::NVE_BASE_RESET_CSS;
use crate::stylesheet::{CachedRule, StyleSheetCache, STATE_TIER_COUNT};

#[wasm_bindgen::prelude::wasm_bindgen(inline_js = "
export function nve_mount_adopted_sheets(doc, base, structural, media, s0, s1, s2, s3, s4) {
  try {
    if (!doc || !('adoptedStyleSheets' in doc)) return false;
    doc.adoptedStyleSheets = [
      ...Array.from(doc.adoptedStyleSheets),
      base, structural, media, s0, s1, s2, s3, s4,
    ];
    return true;
  } catch (_) {
    return false;
  }
}
export function nve_unmount_adopted_sheets(doc, base, structural, media, s0, s1, s2, s3, s4) {
  try {
    if (!doc || !('adoptedStyleSheets' in doc)) return;
    const drop = new Set([base, structural, media, s0, s1, s2, s3, s4]);
    doc.adoptedStyleSheets = Array.from(doc.adoptedStyleSheets).filter(s => !drop.has(s));
  } catch (_) {}
}
export function nve_append_css_rule(sheet, cssText) {
  try {
    sheet.insertRule(cssText, sheet.cssRules.length);
  } catch (_) {}
}
")]
extern "C" {
    fn nve_mount_adopted_sheets(
        doc: &web_sys::Document,
        base: &web_sys::CssStyleSheet,
        structural: &web_sys::CssStyleSheet,
        media: &web_sys::CssStyleSheet,
        s0: &web_sys::CssStyleSheet,
        s1: &web_sys::CssStyleSheet,
        s2: &web_sys::CssStyleSheet,
        s3: &web_sys::CssStyleSheet,
        s4: &web_sys::CssStyleSheet,
    ) -> bool;
    fn nve_unmount_adopted_sheets(
        doc: &web_sys::Document,
        base: &web_sys::CssStyleSheet,
        structural: &web_sys::CssStyleSheet,
        media: &web_sys::CssStyleSheet,
        s0: &web_sys::CssStyleSheet,
        s1: &web_sys::CssStyleSheet,
        s2: &web_sys::CssStyleSheet,
        s3: &web_sys::CssStyleSheet,
        s4: &web_sys::CssStyleSheet,
    );
    fn nve_append_css_rule(sheet: &web_sys::CssStyleSheet, css_text: &str);
}

impl Drop for StyleSheetCache {
    fn drop(&mut self) {
        self.clear_dom_sheets();
    }
}

impl StyleSheetCache {
    pub(crate) fn clear_dom_sheets(&mut self) {
        self.pending_structural.clear();
        self.pending_media.clear();
        for tier in &mut self.pending_state {
            tier.clear();
        }
        if self.adopted_mounted
            && let (
                Some(doc),
                Some(base),
                Some(structural),
                Some(media),
                Some(s0),
                Some(s1),
                Some(s2),
                Some(s3),
                Some(s4),
            ) = (
                web_sys::window().and_then(|w| w.document()),
                self.base_sheet.as_ref(),
                self.structural_sheet.as_ref(),
                self.media_sheet.as_ref(),
                self.state_sheets[0].as_ref(),
                self.state_sheets[1].as_ref(),
                self.state_sheets[2].as_ref(),
                self.state_sheets[3].as_ref(),
                self.state_sheets[4].as_ref(),
            )
        {
            nve_unmount_adopted_sheets(&doc, base, structural, media, s0, s1, s2, s3, s4);
        }
        self.base_sheet = None;
        self.structural_sheet = None;
        self.media_sheet = None;
        self.state_sheets = Default::default();
        self.adopted_mounted = false;
        self.fallback_mounted = false;
    }
    /// Flushes all pending compiled CSS rules into live browser stylesheets in cascade order.
    pub fn flush_to_dom(&mut self, doc: &web_sys::Document) {
        if !self.adopted_mounted && !self.fallback_mounted {
            if self.try_mount_adopted(doc) {
                self.adopted_mounted = true;
            } else {
                self.fallback_mounted = true;
                self.mount_fallback_base(doc);
            }
        }

        let structural = std::mem::take(&mut self.pending_structural);
        let media = std::mem::take(&mut self.pending_media);
        let mut states: [Vec<CachedRule>; STATE_TIER_COUNT] = Default::default();
        for (dst, src) in states.iter_mut().zip(self.pending_state.iter_mut()) {
            *dst = std::mem::take(src);
        }

        if self.adopted_mounted {
            if let Some(sheet) = self.structural_sheet.as_ref() {
                insert_rules(sheet, &structural);
            }
            if let Some(sheet) = self.media_sheet.as_ref() {
                insert_rules(sheet, &media);
            }
            for (tier, batch) in states.iter().enumerate() {
                if let Some(sheet) = self.state_sheets[tier].as_ref() {
                    insert_rules(sheet, batch);
                }
            }
        } else {
            append_fallback_rules(doc, "structural", &structural);
            append_fallback_rules(doc, "media", &media);
            for (tier, batch) in states.iter().enumerate() {
                let tier_key = match tier {
                    0 => "state-0",
                    1 => "state-1",
                    2 => "state-2",
                    3 => "state-3",
                    _ => "state-4",
                };
                append_fallback_rules(doc, tier_key, batch);
            }
        }
    }

    fn try_mount_adopted(&mut self, doc: &web_sys::Document) -> bool {
        let has_dom_base = doc
            .query_selector("style[data-nve-base]")
            .ok()
            .flatten()
            .is_some();
        let has_dom_ssr = doc
            .query_selector("style[data-nve-ssr]")
            .ok()
            .flatten()
            .is_some();
        if !has_dom_base
            && has_dom_ssr
            && let (Ok(Some(ssr_el)), Ok(base_el)) =
                (doc.query_selector("style[data-nve-ssr]"), doc.create_element("style"))
            && let Some(parent) = ssr_el.parent_node()
        {
            let _ = base_el.set_attribute("data-nve-base", "");
            base_el.set_text_content(Some(NVE_BASE_RESET_CSS));
            let _ = parent.insert_before(&base_el, Some(&ssr_el));
        }

        let Ok(base) = web_sys::CssStyleSheet::new() else {
            return false;
        };
        // Only populate the adopted base sheet on fresh client mounts; during SSR/snapshot
        // hydration `<style data-nve-base>` precedes `<style data-nve-ssr>` in `<head>`, whereas
        // `adoptedStyleSheets` always cascades *after* `<style>` tags and would otherwise clobber
        // equal-specificity SSR structural rules.
        if !has_dom_base && !has_dom_ssr {
            let _ = base.replace_sync(NVE_BASE_RESET_CSS);
        }
        let Ok(structural) = web_sys::CssStyleSheet::new() else {
            return false;
        };
        let Ok(media) = web_sys::CssStyleSheet::new() else {
            return false;
        };
        let Ok(s0) = web_sys::CssStyleSheet::new() else {
            return false;
        };
        let Ok(s1) = web_sys::CssStyleSheet::new() else {
            return false;
        };
        let Ok(s2) = web_sys::CssStyleSheet::new() else {
            return false;
        };
        let Ok(s3) = web_sys::CssStyleSheet::new() else {
            return false;
        };
        let Ok(s4) = web_sys::CssStyleSheet::new() else {
            return false;
        };

        if !nve_mount_adopted_sheets(doc, &base, &structural, &media, &s0, &s1, &s2, &s3, &s4) {
            return false;
        }

        self.base_sheet = Some(base);
        self.structural_sheet = Some(structural);
        self.media_sheet = Some(media);
        self.state_sheets = [Some(s0), Some(s1), Some(s2), Some(s3), Some(s4)];
        true
    }

    fn mount_fallback_base(&self, doc: &web_sys::Document) {
        if doc
            .query_selector("style[data-nve-base]")
            .ok()
            .flatten()
            .is_some()
        {
            return;
        }
        let Ok(Some(head)) = doc.query_selector("head") else {
            return;
        };
        if let Ok(el) = doc.create_element("style") {
            let _ = el.set_attribute("data-nve-base", "");
            let _ = el.set_attribute("data-nve-tier", "base");
            el.set_text_content(Some(NVE_BASE_RESET_CSS));
            if let Ok(Some(ssr_el)) = doc.query_selector("style[data-nve-ssr]")
                && let Some(parent) = ssr_el.parent_node()
            {
                let _ = parent.insert_before(&el, Some(&ssr_el));
            } else {
                let _ = head.append_child(&el);
            }
        }
    }
}

fn insert_rules(sheet: &web_sys::CssStyleSheet, rules: &[CachedRule]) {
    for rule in rules {
        nve_append_css_rule(sheet, &rule.css_rule);
    }
}

fn append_fallback_rules(doc: &web_sys::Document, tier: &str, rules: &[CachedRule]) {
    if rules.is_empty() {
        return;
    }
    let selector = format!("style[data-nve-tier=\"{tier}\"]");
    let el = if let Ok(Some(existing)) = doc.query_selector(&selector) {
        existing
    } else if let (Ok(Some(head)), Ok(created)) =
        (doc.query_selector("head"), doc.create_element("style"))
    {
        let _ = created.set_attribute("data-nve-tier", tier);
        let _ = head.append_child(&created);
        created
    } else {
        return;
    };
    let mut text = el.text_content().unwrap_or_default();
    for rule in rules {
        text.push_str(&rule.css_rule);
    }
    el.set_text_content(Some(&text));
}
