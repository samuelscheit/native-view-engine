//! Content-addressed deduplicated structural, `@media`, `:hover`/`:active`/`:focus`/`:disabled`,
//! cursor, and transition stylesheet cache directly over `NodeStore` word tables.

use std::collections::BTreeMap;

use nve_abi::hash::HashSet;
use nve_components::interaction::CursorKind;
use nve_components::tables::{
    Interaction, Motion, Responsive, ResponsiveRules, StateRules, Transitions,
};
use nve_store::{NodeId, NodeStore};

use crate::css::{format_content_class, write_store_node_css};
use crate::media_css::{compile_node_media_rules, CompiledMediaRule};
use crate::state_css::{compile_node_state_rules, format_cursor_rule, CompiledStateRule};
use crate::transition_css::compile_node_transitions;

/// Maximum number of deduplicated dynamic CSS rules emitted before falling back to inline styles.
pub const MAX_DYNAMIC_RULES: usize = 8192;
/// Number of ordered interactive cascade tiers (`Hover < Focus < FocusVisible < Active < Disabled`).
pub const STATE_TIER_COUNT: usize = 5;
/// Batch threshold before rewriting a fallback `<style>` element wholesale on `wasm32`.
pub const INCREMENTAL_RULE_BATCH_THRESHOLD: usize = 32;

/// One compiled stylesheet entry in [`StyleSheetCache`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedRule {
    pub class_name: String,
    pub css_rule: String,
}

/// Resolved class list + newly generated rules for a single `NodeStore` node.
#[derive(Debug, Clone, Default)]
pub struct ResolvedNodeStyle {
    pub classes: String,
    pub inline_fallback: String,
    pub structural_rule: Option<(String, String)>,
    pub cursor_rule: Option<(&'static str, &'static str)>,
    pub transition_rule: Option<(String, String)>,
    pub media_rules: Vec<CompiledMediaRule>,
    pub state_rules: Vec<CompiledStateRule>,
}

/// Content-addressed CSS stylesheet cache shared between SSR and live `wasm32` DOM.
#[derive(Default)]
pub struct StyleSheetCache {
    pub(crate) structural: BTreeMap<String, String>,
    pub(crate) media: Vec<CompiledMediaRule>,
    pub(crate) media_seen: HashSet<String>,
    pub(crate) state: [Vec<CompiledStateRule>; STATE_TIER_COUNT],
    pub(crate) state_seen: HashSet<String>,
    #[cfg(target_arch = "wasm32")]
    pub(crate) base_sheet: Option<web_sys::CssStyleSheet>,
    #[cfg(target_arch = "wasm32")]
    pub(crate) structural_sheet: Option<web_sys::CssStyleSheet>,
    #[cfg(target_arch = "wasm32")]
    pub(crate) media_sheet: Option<web_sys::CssStyleSheet>,
    #[cfg(target_arch = "wasm32")]
    pub(crate) state_sheets: [Option<web_sys::CssStyleSheet>; STATE_TIER_COUNT],
    #[cfg(target_arch = "wasm32")]
    pub(crate) pending_structural: Vec<CachedRule>,
    #[cfg(target_arch = "wasm32")]
    pub(crate) pending_media: Vec<CachedRule>,
    #[cfg(target_arch = "wasm32")]
    pub(crate) pending_state: [Vec<CachedRule>; STATE_TIER_COUNT],
    #[cfg(target_arch = "wasm32")]
    pub(crate) fallback_mounted: bool,
    #[cfg(target_arch = "wasm32")]
    pub(crate) adopted_mounted: bool,
}

impl StyleSheetCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self) {
        self.structural.clear();
        self.media.clear();
        self.media_seen.clear();
        for tier in &mut self.state {
            tier.clear();
        }
        self.state_seen.clear();
        #[cfg(target_arch = "wasm32")]
        self.clear_dom_sheets();
    }

    pub fn rule_count(&self) -> usize {
        self.structural.len()
            + self.media.len()
            + self.state.iter().map(Vec::len).sum::<usize>()
    }

    /// Pre-seeds known SSR class hashes so live hydration never re-inserts rules already in
    /// `<style data-nve-ssr>`.
    pub fn seed_ssr_hashes<'a>(&mut self, hashes: impl IntoIterator<Item = &'a str>) {
        for h in hashes {
            let trimmed = h.trim();
            if !trimmed.is_empty() {
                self.structural
                    .entry(trimmed.to_owned())
                    .or_default();
                self.media_seen.insert(trimmed.to_owned());
                self.state_seen.insert(trimmed.to_owned());
            }
        }
    }

    /// Resolves classes for `id` in `store`, registering any unseen rules into the cache.
    pub fn resolve_node(&mut self, store: &NodeStore, id: NodeId) -> ResolvedNodeStyle {
        let allow_new = self.rule_count() < MAX_DYNAMIC_RULES;
        let resolved = resolve_store_node_style(store, id, allow_new, |cls| {
            self.structural.contains_key(cls)
        });
        if let Some((ref cls, ref decls)) = resolved.structural_rule
            && !self.structural.contains_key(cls)
        {
            self.structural.insert(cls.clone(), decls.clone());
            #[cfg(target_arch = "wasm32")]
            self.pending_structural.push(CachedRule {
                class_name: cls.clone(),
                css_rule: format!("@layer nve-s{{.{cls}{{{decls}}}}}"),
            });
        }
        if let Some((cls, decls)) = resolved.cursor_rule
            && !self.structural.contains_key(cls)
        {
            self.structural.insert(cls.to_owned(), decls.to_owned());
            #[cfg(target_arch = "wasm32")]
            self.pending_structural.push(CachedRule {
                class_name: cls.to_owned(),
                css_rule: format!("@layer nve-s{{.{cls}{{{decls}}}}}"),
            });
        }
        if let Some((ref cls, ref decls)) = resolved.transition_rule
            && !self.structural.contains_key(cls)
        {
            self.structural.insert(cls.clone(), decls.clone());
            #[cfg(target_arch = "wasm32")]
            self.pending_structural.push(CachedRule {
                class_name: cls.clone(),
                css_rule: format!("@layer nve-s{{.{cls}{{{decls}}}}}"),
            });
        }
        for mr in &resolved.media_rules {
            if self.media_seen.insert(mr.class_name.clone()) {
                #[cfg(target_arch = "wasm32")]
                self.pending_media.push(CachedRule {
                    class_name: mr.class_name.clone(),
                    css_rule: format!("@layer nve-m{{{}}}", mr.to_css_rule()),
                });
                self.media.push(mr.clone());
            }
        }
        for sr in &resolved.state_rules {
            if self.state_seen.insert(sr.class_name.clone()) {
                let tier = sr.state.cascade_tier().unwrap_or(0).min(STATE_TIER_COUNT - 1);
                #[cfg(target_arch = "wasm32")]
                self.pending_state[tier].push(CachedRule {
                    class_name: sr.class_name.clone(),
                    css_rule: format!("@layer nve-st{tier}{{{}}}", sr.to_css_rule()),
                });
                self.state[tier].push(sr.clone());
            }
        }
        resolved
    }

    /// Serializes all accumulated rules (structural, then `@media`, then ordered state tiers)
    /// into CSS Cascade Layers so SSR `<style data-nve-ssr>` and live `adoptedStyleSheets`
    /// maintain identical tier ordering (`nve-base < nve-s < nve-m < nve-st0..4`).
    pub fn serialize_css(&self) -> (String, Vec<String>) {
        use core::fmt::Write as _;
        let mut css = String::new();
        let mut hashes = Vec::with_capacity(self.rule_count());
        let mut has_structural = false;
        for (cls, decls) in &self.structural {
            if !decls.is_empty() {
                if !has_structural {
                    css.push_str("@layer nve-s{");
                    has_structural = true;
                }
                css.push('.');
                css.push_str(cls);
                css.push('{');
                css.push_str(decls);
                css.push('}');
                hashes.push(cls.clone());
            }
        }
        if has_structural {
            css.push('}');
        }
        if !self.media.is_empty() {
            css.push_str("@layer nve-m{");
            for mr in &self.media {
                mr.write_css_rule_into(&mut css);
                hashes.push(mr.class_name.clone());
            }
            css.push('}');
        }
        for (tier_idx, tier) in self.state.iter().enumerate() {
            if tier.is_empty() {
                continue;
            }
            let _ = write!(css, "@layer nve-st{tier_idx}{{");
            for sr in tier {
                sr.write_css_rule_into(&mut css);
                hashes.push(sr.class_name.clone());
            }
            css.push('}');
        }
        (css, hashes)
    }
}

fn cursor_class_and_decl(c: CursorKind) -> Option<(&'static str, &'static str)> {
    format_cursor_rule(c)
}

/// Computes content-addressed classes for `id` directly from `NodeStore` word tables.
pub fn resolve_store_node_style(
    store: &NodeStore,
    id: NodeId,
    allow_new: bool,
    has_rule: impl Fn(&str) -> bool,
) -> ResolvedNodeStyle {
    let mut out = ResolvedNodeStyle::default();
    if id == NodeId::ROOT {
        out.classes.push_str("nve nve-r");
    } else {
        out.classes.push_str("nve");
    }

    let mut decls = String::new();
    write_store_node_css(store, id, false, &mut decls);
    if !decls.is_empty() {
        let cls = format_content_class("n", &[decls.as_bytes()]);
        if allow_new || has_rule(&cls) {
            out.classes.push(' ');
            out.classes.push_str(&cls);
            out.structural_rule = Some((cls, decls));
        } else {
            out.inline_fallback = decls;
        }
    }

    if let Some(inter) = store.try_row::<Interaction>(id) {
        if let Some((cls, decl)) = cursor_class_and_decl(inter.cursor()) {
            out.classes.push(' ');
            out.classes.push_str(cls);
            out.cursor_rule = Some((cls, decl));
        }
        let sr_res = inter.state_rules();
        if !sr_res.is_none()
            && let Some(rules) = store.resource::<StateRules>(sr_res)
        {
            let start = out.state_rules.len();
            compile_node_state_rules(&rules.0, &mut out.state_rules);
            for compiled in &out.state_rules[start..] {
                out.classes.push(' ');
                out.classes.push_str(&compiled.class_name);
            }
        }
    }

    if let Some(resp) = store.try_row::<Responsive>(id) {
        let rr_res = resp.rules();
        let rules_slice: &[nve_components::media::ResponsiveRule] = if !rr_res.is_none() {
            store
                .resource::<ResponsiveRules>(rr_res)
                .map(|r| &*r.0)
                .unwrap_or(&[])
        } else {
            &[]
        };
        let start = out.media_rules.len();
        compile_node_media_rules(resp.show_on(), rules_slice, &mut out.media_rules);
        for compiled in &out.media_rules[start..] {
            out.classes.push(' ');
            out.classes.push_str(&compiled.class_name);
        }
    }

    if let Some(motion) = store.try_row::<Motion>(id) {
        let tr_res = motion.transitions();
        if !tr_res.is_none()
            && let Some(specs) = store.resource::<Transitions>(tr_res)
            && let Some(tr) = compile_node_transitions(&specs.0)
        {
            out.classes.push(' ');
            out.classes.push_str(&tr.class_name);
            out.transition_rule = Some((tr.class_name, tr.decls));
            if let Some(rm) = tr.reduced_motion_rule {
                out.media_rules.push(rm);
            }
        }
    }

    out
}
