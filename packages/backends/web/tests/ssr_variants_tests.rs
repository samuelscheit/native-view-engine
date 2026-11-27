//! `ssr_style_overrides`: restyling a base SSR render into another appearance by content class,
//! per node when a shared base class diverges, and refusing structurally different trees.

use nve_backend_web::ssr_variants::{ssr_style_overrides, write_scoped_overrides, SsrVariantMismatch};
use nve_backend_web::WebBackend;
use nve_components::tables::Visual;
use nve_store::{ChangeSet, NodeId, NodeStore, Rgba};

/// Root with one child per background; equal colors share one SSR content class.
fn tree(backgrounds: &[(u8, u8, u8)]) -> NodeStore {
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let mut tx = store.tx();
    let nodes: Vec<NodeId> = backgrounds.iter().map(|_| tx.create()).collect();
    tx.set_children(NodeId::ROOT, &nodes);
    for (&n, &(r, g, b)) in nodes.iter().zip(backgrounds) {
        tx.edit::<Visual>(n).background(Rgba::rgb(r, g, b)).border_radius(4.0);
    }
    tx.commit_into(&mut changes);
    store
}

const DARK: (u8, u8, u8) = (9, 9, 11);
const LIGHT: (u8, u8, u8) = (246, 246, 248);
const ACCENT: (u8, u8, u8) = (79, 70, 229);

#[test]
fn identical_appearances_need_no_overrides() {
    let base = tree(&[DARK, ACCENT]);
    assert_eq!(ssr_style_overrides(&base, &tree(&[DARK, ACCENT])), Ok(Vec::new()));
}

#[test]
fn nodes_sharing_a_class_are_restyled_by_that_class_once() {
    let base = tree(&[DARK, DARK, ACCENT]);
    let rules = ssr_style_overrides(&base, &tree(&[LIGHT, LIGHT, ACCENT])).expect("same tree");
    assert_eq!(rules.len(), 1, "{rules:?}");
    assert!(rules[0].selector.starts_with(".n"), "{rules:?}");
    assert!(rules[0].declarations.contains("background:#f6f6f8"), "{rules:?}");

    // The selector is the very class the base SSR HTML carries on both nodes.
    let html = WebBackend::new().render_ssr(&base).html;
    assert_eq!(html.matches(&rules[0].selector[1..]).count(), 2, "{html}");
}

#[test]
fn a_shared_base_class_that_diverges_falls_back_to_node_ids() {
    // Both nodes are DARK in the base, but only the first changes in the variant.
    let base = tree(&[DARK, DARK]);
    let rules = ssr_style_overrides(&base, &tree(&[LIGHT, DARK])).expect("same tree");
    assert_eq!(rules.len(), 1, "{rules:?}");
    assert!(rules[0].selector.starts_with("[data-nve-id=\""), "{rules:?}");
    let html = WebBackend::new().render_ssr(&base).html;
    let attr = &rules[0].selector[1..rules[0].selector.len() - 1];
    assert!(html.contains(attr), "{attr} missing from {html}");
}

#[test]
fn properties_only_the_base_declares_are_unset() {
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let mut tx = store.tx();
    let n = tx.create();
    tx.set_children(NodeId::ROOT, &[n]);
    tx.edit::<Visual>(n).background(Rgba::rgb(DARK.0, DARK.1, DARK.2)).border_radius(4.0);
    tx.commit_into(&mut changes);
    let rules = ssr_style_overrides(&store, &tree(&[])).err();
    assert!(rules.is_some(), "different trees are rejected");

    // Same tree; the variant drops the radius.
    let mut variant = NodeStore::new();
    let mut tx = variant.tx();
    let v = tx.create();
    tx.set_children(NodeId::ROOT, &[v]);
    tx.edit::<Visual>(v).background(Rgba::rgb(LIGHT.0, LIGHT.1, LIGHT.2));
    tx.commit_into(&mut changes);
    let rules = ssr_style_overrides(&store, &variant).expect("same tree");
    assert_eq!(rules.len(), 1, "{rules:?}");
    assert!(rules[0].declarations.contains("border-radius:unset;"), "{rules:?}");
    assert!(rules[0].declarations.contains("background:#f6f6f8"), "{rules:?}");
}

#[test]
fn overrides_carry_only_the_changed_properties() {
    let rules = ssr_style_overrides(&tree(&[DARK]), &tree(&[LIGHT])).expect("same tree");
    assert_eq!(rules.len(), 1, "{rules:?}");
    assert_eq!(rules[0].declarations, "background:#f6f6f8;", "the unchanged radius stays on the base class");
}

#[test]
fn structurally_different_variants_are_rejected() {
    let base = tree(&[DARK, DARK]);
    assert_eq!(ssr_style_overrides(&base, &tree(&[DARK])), Err(SsrVariantMismatch { node: NodeId::ROOT }));
}

#[test]
fn scoped_output_prefixes_every_selector_and_merges_equal_declarations() {
    let base = tree(&[DARK, ACCENT]);
    let rules = ssr_style_overrides(&base, &tree(&[LIGHT, LIGHT])).expect("same tree");
    assert_eq!(rules.len(), 2);
    let mut css = String::new();
    write_scoped_overrides(&rules, "html.x", &mut css);
    assert_eq!(css.matches('{').count(), 1, "equal declarations share one rule: {css}");
    assert_eq!(css.matches("html.x .n").count(), 2, "{css}");
}
