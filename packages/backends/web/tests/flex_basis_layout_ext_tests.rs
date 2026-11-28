#![cfg(feature = "ssr")]
//! `flex-basis` is the typed `Layout::flex_basis` (never an HTML attribute): each distinct basis
//! hashes into its own structural class emitted without any base-reset override, `auto`
//! emits nothing, and a cleared basis falls back to the `auto` class.

mod support;

use nve_backend_web::NVE_BASE_RESET_CSS;
use nve_tables::tables::Layout;
use nve_store::{Length, NodeId, NodeStore};

use support::*;

fn class_of(html: &str, id: u32) -> String {
    let at = html.find(&format!("data-nve-id=\"{id}\"")).expect("node in html");
    let tag_start = html[..at].rfind('<').expect("tag start");
    let tag = &html[tag_start..at + html[at..].find('>').expect("tag end")];
    let cls = tag.split("class=\"").nth(1).expect("class attr");
    cls[..cls.find('"').expect("class end")].to_owned()
}

#[test]
fn typed_flex_basis_hashes_emits_and_resets_without_raw_attrs() {
    let mut web = web();
    let mut store = NodeStore::new();
    let bases = [
        (2, Length::points(280.0), "280px"),
        (3, Length::points(110.0), "110px"),
        // `0%` and the length `0` differ for indefinite containers: distinct rules.
        (4, Length::percent(0.0), "0%"),
        (5, Length::points(0.0), "0"),
    ];
    commit(&mut web, &mut store, |tx| {
        for &(id, basis, _) in &bases {
            let n = node(tx, id);
            tx.edit::<Layout>(n).flex_basis(basis);
        }
        let auto = node(tx, 6);
        tx.edit::<Layout>(auto).flex_basis(Length::AUTO);
        tx.set_children(NodeId::ROOT, &[nid(2), nid(3), nid(4), nid(5), auto]);
    });

    let ssr = web.render_ssr(&store);
    for (_, _, css) in bases {
        assert_eq!(
            ssr.style_css.matches(&format!("flex-basis:{css};")).count(),
            1,
            "each distinct basis needs exactly one structural rule: {}",
            ssr.style_css
        );
    }
    // The base reset never sets a basis for `.nve` nodes, so a node's longhand is never overridden.
    assert!(!NVE_BASE_RESET_CSS.contains("flex-basis"));
    assert!(!ssr.style_css.contains("flex:"), "{}", ssr.style_css);
    assert_eq!(ssr.style_css.matches("flex-basis:").count(), 4, "auto emits nothing");
    assert!(!ssr.html.contains("flex-basis="), "{}", ssr.html);
    assert!(!ssr.html.contains("flexBasis="), "{}", ssr.html);

    // Clearing the basis drops it from the node's class; re-basing reuses the existing class.
    let before = web.render_ssr(&store).html;
    commit(&mut web, &mut store, |tx| {
        tx.edit::<Layout>(nid(2)).flex_basis(Length::AUTO);
        tx.edit::<Layout>(nid(3)).flex_basis(Length::points(280.0));
    });
    let after = web.render_ssr(&store);
    assert_ne!(before, after.html, "node classes change with the basis");
    assert_eq!(class_of(&after.html, 2), class_of(&after.html, 6), "cleared == auto");
    assert_eq!(class_of(&after.html, 3), class_of(&before, 2), "same basis, same class");
}
