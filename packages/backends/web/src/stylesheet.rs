//! Option 5 Hybrid CSS stylesheet cache: compact declaration emitters, structural vs volatile
//! lane resolution, bounded `MAX_DYNAMIC_RULES`, and 3-layer cascade-ordered flushing
//! (base structural -> `@media` -> 5-tier interactive pseudo-state rules).

use nve_abi::hash::{HashMap, HashSet};
use nve_abi::{ComponentKind, EventMask};

pub use crate::css::{
    write_compact_cursor_into, write_compact_layout_into, write_compact_style_into,
    write_volatile_inline_into,
};
use crate::css::MirrorElement;
use crate::layout_ext_css::{write_compact_layout_ext_into, write_compact_transform_into};
use crate::media_css::{compile_node_media_rules, CompiledMediaRule};
use crate::paint_svg::write_compact_background_fill_into;
use crate::state_css::{compile_node_state_rules, format_cursor_rule, CompiledStateRule};
use crate::style_hash::{
    format_structural_class, hash_structural_style_full, is_inheriting_leaf_text,
};
use crate::transition_css::compile_node_transitions;
use nve_components::motion::TransitionSpec;

/// Hard upper bound on dynamic structural CSS rules in [`StyleSheetCache`].
pub const MAX_DYNAMIC_RULES: usize = 4096;

/// Incremental rule batch size threshold above which `wasm32` allocates a new chunked
/// `CSSStyleSheet` via `replace_sync` instead of per-rule `insert_rule`.
pub const INCREMENTAL_RULE_BATCH_THRESHOLD: usize = 16;

#[cfg(target_arch = "wasm32")]
pub(crate) const STATE_TIER_COUNT: usize = 5;

/// Resolved Option 5 Hybrid presentation for a single DOM element.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResolvedNodeStyle {
    pub class_name: String,
    pub inline_style: String,
    pub structural_rule: Option<(String, String)>,
    pub cursor_rule: Option<(&'static str, &'static str)>,
    pub transition_rule: Option<(String, String)>,
    pub media_rules: Vec<CompiledMediaRule>,
    pub state_rules: Vec<CompiledStateRule>,
}

/// Computes the deterministic class list, optional structural rule, base cursor rule, responsive
/// `@media` rules, interactive pseudo-state rules, and optional inline lane declarations for `node`.
pub fn resolve_node_style(
    node: &MirrorElement,
    parent: Option<&MirrorElement>,
    allow_new_rule: bool,
    existing_rule_check: impl Fn(&str) -> bool,
) -> ResolvedNodeStyle {
    let leaf_text = is_inheriting_leaf_text(parent, node);
    let mut class_name = String::with_capacity(48);
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

    let compiled_transition = (!node.transitions.is_empty())
        .then(|| compile_node_transitions(&node.transitions))
        .flatten();
    if let Some(ref tr) = compiled_transition
        && let Some(ref rm_rule) = tr.reduced_motion_rule
        && !media_rules.iter().any(|r| r.class_name == rm_rule.class_name)
    {
        media_rules.push(rm_rule.clone());
    }

    let mut state_rules = Vec::new();
    if !node.state_rules.is_empty() {
        compile_node_state_rules(&node.state_rules, &mut state_rules);
    }

    let mut inline_style = String::new();

    if node.is_consecutive_frame_volatile() {
        write_compact_layout_into(&node.layout, false, &mut inline_style);
        write_compact_layout_ext_into(node.layout_ext.as_deref(), &mut inline_style);
        write_compact_style_into(&node.style, &node.raw_style, leaf_text, false, &mut inline_style);
        write_compact_background_fill_into(node.background_fill.as_ref(), &mut inline_style);
        write_compact_transform_into(node.transform, &mut inline_style);
        write_compact_cursor_into(node.cursor, &mut inline_style);
        TransitionSpec::write_css_list_into(&node.transitions, &mut inline_style);
        if !node.patch_css.is_empty() {
            inline_style.push_str(&node.patch_css);
        }
        for rule in &media_rules {
            class_name.push(' ');
            class_name.push_str(&rule.class_name);
        }
        for rule in &state_rules {
            class_name.push(' ');
            class_name.push_str(&rule.class_name);
        }
        return ResolvedNodeStyle {
            class_name,
            inline_style,
            structural_rule: None,
            cursor_rule: None,
            transition_rule: None,
            media_rules,
            state_rules,
        };
    }

    let mut structural_decls = String::with_capacity(96);
    write_compact_layout_into(&node.layout, true, &mut structural_decls);
    write_compact_layout_ext_into(node.layout_ext.as_deref(), &mut structural_decls);
    write_compact_style_into(
        &node.style,
        &node.raw_style,
        leaf_text,
        true,
        &mut structural_decls,
    );
    write_compact_background_fill_into(node.background_fill.as_ref(), &mut structural_decls);
    write_compact_transform_into(node.transform, &mut structural_decls);

    let structural_rule = if structural_decls.is_empty() {
        None
    } else {
        let hash = hash_structural_style_full(
            &node.layout,
            node.layout_ext.as_deref(),
            &node.style,
            node.background_fill.as_ref(),
            node.transform,
            &node.raw_style,
            leaf_text,
        );
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

    let cursor_rule = node.cursor.and_then(format_cursor_rule).and_then(|(cls, decl)| {
        if existing_rule_check(cls) || allow_new_rule {
            class_name.push(' ');
            class_name.push_str(cls);
            Some((cls, decl))
        } else {
            inline_style.push_str(decl);
            None
        }
    });

    let transition_rule = compiled_transition.and_then(|tr| {
        if existing_rule_check(&tr.class_name) || allow_new_rule {
            class_name.push(' ');
            class_name.push_str(&tr.class_name);
            Some((tr.class_name, tr.decls))
        } else {
            inline_style.push_str(&tr.decls);
            None
        }
    });

    for rule in &media_rules {
        class_name.push(' ');
        class_name.push_str(&rule.class_name);
    }
    for rule in &state_rules {
        class_name.push(' ');
        class_name.push_str(&rule.class_name);
    }

    write_volatile_inline_into(&node.layout, &node.style, &node.patch_css, &mut inline_style);

    ResolvedNodeStyle {
        class_name,
        inline_style,
        structural_rule,
        cursor_rule,
        transition_rule,
        media_rules,
        state_rules,
    }
}

/// One cached stylesheet rule stored in [`StyleSheetCache`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CachedRule {
    Structural(String),
    Media { condition: String, decls: String },
    State(CompiledStateRule),
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
            Self::State(rule) => rule.write_css_rule_into(out),
        }
    }
}

/// Bounded content-addressed structural, `@media`, and interactive pseudo-state CSS cache with
/// cascade-ordered flushing (`structural -> @media -> Hover < Focus < FocusVisible < Active < Disabled`).
#[derive(Default)]
pub struct StyleSheetCache {
    rules: HashMap<String, String>,
    media_rules: HashMap<String, CompiledMediaRule>,
    state_rules: HashMap<String, CompiledStateRule>,
    pub(crate) inserted: HashSet<String>,
    pub(crate) pending: Vec<(String, CachedRule)>,
    pub(crate) base_mounted: bool,
    #[cfg(not(target_arch = "wasm32"))]
    flushed_css_rules: Vec<String>,
    #[cfg(target_arch = "wasm32")]
    pub(crate) initial_flushed: bool,
    #[cfg(target_arch = "wasm32")]
    pub(crate) use_adopted: Option<bool>,
    #[cfg(target_arch = "wasm32")]
    pub(crate) base_sheet: Option<web_sys::CssStyleSheet>,
    #[cfg(target_arch = "wasm32")]
    pub(crate) dynamic_sheets: Vec<web_sys::CssStyleSheet>,
    #[cfg(target_arch = "wasm32")]
    pub(crate) dynamic_sheet_last_len: u32,
    #[cfg(target_arch = "wasm32")]
    pub(crate) media_sheets: Vec<web_sys::CssStyleSheet>,
    #[cfg(target_arch = "wasm32")]
    pub(crate) media_sheet_last_len: u32,
    #[cfg(target_arch = "wasm32")]
    pub(crate) state_sheets: [Vec<web_sys::CssStyleSheet>; STATE_TIER_COUNT],
    #[cfg(target_arch = "wasm32")]
    pub(crate) state_sheet_last_lens: [u32; STATE_TIER_COUNT],
    #[cfg(target_arch = "wasm32")]
    pub(crate) fallback_style_el: Option<web_sys::Element>,
    #[cfg(target_arch = "wasm32")]
    pub(crate) fallback_media_style_el: Option<web_sys::Element>,
    #[cfg(target_arch = "wasm32")]
    pub(crate) fallback_state_style_els: [Option<web_sys::Element>; STATE_TIER_COUNT],
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
            || self.state_rules.contains_key(class_name)
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

    /// Queues a content-addressed interactive pseudo-state rule (`nve-s-XXXXXXXX`) unless already
    /// cached or seeded.
    pub fn ensure_state_rule(&mut self, rule: CompiledStateRule) -> bool {
        if self.contains_rule(&rule.class_name) {
            return true;
        }
        if !self.can_insert_new() {
            return false;
        }
        let key = rule.class_name.clone();
        let cached = CachedRule::State(rule.clone());
        self.state_rules.insert(key.clone(), rule);
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

    #[inline]
    pub fn state_rules(&self) -> &HashMap<String, CompiledStateRule> {
        &self.state_rules
    }

    /// Returns all CSS rules flushed so far in exact cascade order (structural -> `@media` ->
    /// state rules ordered `Hover < Focus < FocusVisible < Active < Disabled`) on native targets.
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
        let mut state: Vec<(String, CompiledStateRule)> = Vec::new();
        for (cls, rule) in batch {
            self.inserted.insert(cls.clone());
            match rule {
                CachedRule::Structural(_) => structural.push((cls, rule)),
                CachedRule::Media { .. } => media.push((cls, rule)),
                CachedRule::State(sr) => state.push((cls, sr)),
            }
        }
        state.sort_by_key(|(_, sr)| sr.state.cascade_order());
        for (cls, rule) in structural
            .into_iter()
            .chain(media)
            .chain(state.into_iter().map(|(cls, sr)| (cls, CachedRule::State(sr))))
        {
            let mut css = String::with_capacity(64);
            rule.write_css_into(&cls, &mut css);
            self.flushed_css_rules.push(css);
        }
    }
}
