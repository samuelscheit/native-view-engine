//! Appearance variants for static SSR.
//!
//! A static page is rendered once (the *base* appearance), but its first paint must already match
//! the visitor's appearance (light/dark, increased contrast) before the client hydrates. Rendering
//! the same tree again under another appearance and diffing each node's resolved structural
//! declarations yields override rules that hosts emit unlayered (so they beat the base
//! `@layer nve-s` rules) inside the matching `@media` condition. Hydration is unaffected: the
//! overrides only restyle, and tags and node ids stay identical.
//!
//! Hover/focus state rules and responsive patches keep their base colors until hydration.

use std::collections::BTreeMap;

use nve_store::{NodeId, NodeStore};

use crate::stylesheet::resolve_store_node_style;
use crate::table_system::WebTableRegistry;

/// One override rule: a selector matching SSR-rendered nodes (`.n1a2b3c4d` content class, or
/// `[data-nve-id="N"]` when nodes sharing a base class diverge) and the declarations they take
/// under the variant appearance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SsrStyleOverride {
    pub selector: String,
    pub declarations: String,
}

/// The variant tree is not the base tree (different children at `node`), so node-keyed overrides
/// would restyle the wrong elements.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SsrVariantMismatch {
    pub node: NodeId,
}

impl std::fmt::Display for SsrVariantMismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SSR appearance variant diverges structurally at node {}", self.node.raw())
    }
}

impl std::error::Error for SsrVariantMismatch {}

fn declarations(decls: &str) -> impl Iterator<Item = (&str, &str)> {
    decls.split(';').filter_map(|d| d.split_once(':')).map(|(p, v)| (p.trim(), v))
}

/// Only what the variant changes: its properties whose value differs from `base` (the base class
/// keeps applying the rest), plus `prop:unset` for every property only `base` declares.
fn override_declarations(base: &str, variant: &str) -> String {
    let mut out = String::new();
    for (prop, value) in declarations(variant) {
        if !declarations(base).any(|(p, v)| p == prop && v == value) {
            out.push_str(prop);
            out.push(':');
            out.push_str(value);
            out.push(';');
        }
    }
    for (prop, _) in declarations(base) {
        if !declarations(variant).any(|(p, _)| p == prop) {
            out.push_str(prop);
            out.push_str(":unset;");
        }
    }
    out
}

fn structural_rule(store: &NodeStore, id: NodeId, systems: &WebTableRegistry) -> (Option<String>, String) {
    resolve_store_node_style(store, id, systems, true, |_| true)
        .structural_rule
        .map_or((None, String::new()), |(cls, decls)| (Some(cls), decls))
}

/// Override rules that restyle `base`'s SSR output into `variant`'s appearance. Both stores must
/// hold the same tree (same node ids and children); only declarations may differ.
pub fn ssr_style_overrides(base: &NodeStore, variant: &NodeStore) -> Result<Vec<SsrStyleOverride>, SsrVariantMismatch> {
    // Per base class: every node's (id, variant declarations, changed?).
    let systems = WebTableRegistry::new();
    let mut by_class: BTreeMap<String, Vec<(NodeId, String, bool)>> = BTreeMap::new();
    let mut by_node: Vec<(NodeId, String)> = Vec::new();
    let mut stack = vec![NodeId::ROOT];
    while let Some(id) = stack.pop() {
        if !variant.is_live(id) || base.children(id) != variant.children(id) {
            return Err(SsrVariantMismatch { node: id });
        }
        stack.extend_from_slice(base.children(id));
        let (class, base_decls) = structural_rule(base, id, &systems);
        let (_, variant_decls) = structural_rule(variant, id, &systems);
        let changed = base_decls != variant_decls;
        let variant_decls = override_declarations(&base_decls, &variant_decls);
        match class {
            Some(cls) => by_class.entry(cls).or_default().push((id, variant_decls, changed)),
            None if changed => by_node.push((id, variant_decls)),
            None => {}
        }
    }

    let mut out = Vec::new();
    for (class, nodes) in by_class {
        if !nodes.iter().any(|(_, _, changed)| *changed) {
            continue;
        }
        let first = &nodes[0].1;
        if nodes.iter().all(|(_, decls, _)| decls == first) {
            out.push(SsrStyleOverride { selector: format!(".{class}"), declarations: first.clone() });
        } else {
            by_node.extend(nodes.into_iter().filter(|(_, _, changed)| *changed).map(|(id, decls, _)| (id, decls)));
        }
    }
    by_node.sort_by_key(|(id, _)| id.raw());
    out.extend(by_node.into_iter().map(|(id, declarations)| SsrStyleOverride {
        selector: format!("[data-nve-id=\"{}\"]", id.raw()),
        declarations,
    }));
    Ok(out)
}

/// Writes `rules` as CSS with every selector prefixed by `scope` (e.g. `html[data-theme-mode="light"]`),
/// merging rules with identical declarations into one selector list.
pub fn write_scoped_overrides(rules: &[SsrStyleOverride], scope: &str, out: &mut String) {
    let mut by_decls: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for rule in rules {
        by_decls.entry(&rule.declarations).or_default().push(&rule.selector);
    }
    for (decls, selectors) in by_decls {
        for (i, sel) in selectors.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push_str(scope);
            out.push(' ');
            out.push_str(sel);
        }
        out.push('{');
        out.push_str(decls);
        out.push('}');
    }
}
