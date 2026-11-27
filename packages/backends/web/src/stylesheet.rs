//! Content-addressed deduplicated structural, `@media`, `:hover`/`:active`/`:focus`/`:disabled`,
//! cursor, and plugin-contributed (`WebTableSystem::resolve_style`) stylesheet cache directly over
//! `NodeStore` word tables.

use std::collections::BTreeMap;

use nve_abi::hash::HashSet;
use nve_tables::interaction::CursorKind;
use nve_tables::tables::{
    Interaction, Responsive, ResponsiveRules, ResponsiveRulesView, StateRules,
};
use nve_store::{NodeId, NodeStore};

use crate::css::{format_content_class, write_store_node_css};
use crate::media_css::{compile_node_media_rules_view, CompiledMediaRule};
use crate::state_css::{compile_node_state_rules_view, format_cursor_rule, CompiledStateRule};
use crate::table_system::WebTableRegistry;

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
    /// Plugin class rules (`class`, `decls`) contributed via `WebTableSystem::resolve_style`.
    pub class_rules: Vec<(String, String)>,
    pub media_rules: Vec<CompiledMediaRule>,
    pub state_rules: Vec<CompiledStateRule>,
}

impl ResolvedNodeStyle {
    /// Applies the content-addressed class `class` (rule body `decls`) to this node.
    pub fn push_class_rule(&mut self, class: String, decls: String) {
        self.push_class(&class);
        self.class_rules.push((class, decls));
    }

    /// Applies the `@media`-gated rule `rule` to this node.
    pub fn push_media_rule(&mut self, rule: CompiledMediaRule) {
        self.push_class(&rule.class_name);
        self.media_rules.push(rule);
    }

    fn push_class(&mut self, class: &str) {
        if !self.classes.is_empty() {
            self.classes.push(' ');
        }
        self.classes.push_str(class);
    }

    fn remove_class(&mut self, class: &str) {
        let kept: Vec<&str> = self.classes.split(' ').filter(|c| *c != class).collect();
        self.classes = kept.join(" ");
    }

    /// Once the dynamic rule budget is exhausted, drops plugin rules pushed since the
    /// `(class_rules, media_rules)` lengths `since`: class rules not already in the sheet and all
    /// new `@media` rules.
    fn enforce_rule_budget(
        &mut self,
        since: (usize, usize),
        allow_new: bool,
        has_rule: &impl Fn(&str) -> bool,
    ) {
        if allow_new {
            return;
        }
        let rejected: Vec<String> = self.class_rules[since.0..]
            .iter()
            .filter(|(cls, _)| !has_rule(cls))
            .map(|(cls, _)| cls.clone())
            .chain(self.media_rules[since.1..].iter().map(|m| m.class_name.clone()))
            .collect();
        for cls in &rejected {
            self.remove_class(cls);
        }
        self.class_rules.retain(|(cls, _)| !rejected.contains(cls));
        self.media_rules.truncate(since.1);
    }
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

    /// Resolves classes for `id` in `store` (core tables + `systems` plugins), registering any
    /// unseen rules into the cache.
    pub fn resolve_node(
        &mut self,
        store: &NodeStore,
        id: NodeId,
        systems: &WebTableRegistry,
    ) -> ResolvedNodeStyle {
        let allow_new = self.rule_count() < MAX_DYNAMIC_RULES;
        let resolved = resolve_store_node_style(store, id, systems, allow_new, |cls| {
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
        for (cls, decls) in &resolved.class_rules {
            if !self.structural.contains_key(cls) {
                self.structural.insert(cls.clone(), decls.clone());
                #[cfg(target_arch = "wasm32")]
                self.pending_structural.push(CachedRule {
                    class_name: cls.clone(),
                    css_rule: format!("@layer nve-s{{.{cls}{{{decls}}}}}"),
                });
            }
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
    systems: &WebTableRegistry,
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
            compile_node_state_rules_view(rules, Some(store), true, &mut out.state_rules);
            for compiled in &out.state_rules[start..] {
                out.classes.push(' ');
                out.classes.push_str(&compiled.class_name);
            }
        }
    }

    if let Some(resp) = store.try_row::<Responsive>(id) {
        let rr_res = resp.rules();
        let rules_view = if !rr_res.is_none() {
            store
                .resource::<ResponsiveRules>(rr_res)
                .unwrap_or(ResponsiveRulesView::EMPTY)
        } else {
            ResponsiveRulesView::EMPTY
        };
        let start = out.media_rules.len();
        compile_node_media_rules_view(resp.show_on(), rules_view, Some(store), &mut out.media_rules);
        for compiled in &out.media_rules[start..] {
            out.classes.push(' ');
            out.classes.push_str(&compiled.class_name);
        }
    }

    let since = (out.class_rules.len(), out.media_rules.len());
    for sys in systems.iter() {
        if store.has_slot(id, sys.slot()) {
            sys.resolve_style(store, id, &mut out);
        }
    }
    out.enforce_rule_budget(since, allow_new, &has_rule);

    out
}
