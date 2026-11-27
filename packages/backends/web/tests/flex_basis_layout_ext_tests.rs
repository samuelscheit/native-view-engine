#![cfg(feature = "ssr")]
//! `flex-basis` is typed `LayoutExt::flex_basis` (never a raw style prop): each distinct basis
//! hashes into its own structural class emitted after the `.nve{flex:0 1 auto}` reset, `auto`
//! (or a cleared `LayoutExt`) emits nothing, and nothing leaks as an HTML attribute.

use nve_abi::{ComponentKind, Dimension, ROOT_NODE_ID};
use nve_backend_web::{is_raw_style_prop, WebBackend, NVE_BASE_RESET_CSS};
use nve_components::layout_ext::LayoutExt;
use nve_engine::{BatchSink, RenderBackend, RenderSink};

fn basis_ext(basis: Dimension) -> LayoutExt {
    LayoutExt {
        flex_basis: basis,
        ..LayoutExt::default()
    }
}

fn submit(web: &mut WebBackend, frame: u64, build: impl FnOnce(&mut BatchSink)) {
    let mut batch = BatchSink::new(1);
    batch.begin_frame(1, frame);
    build(&mut batch);
    batch.commit_frame(true).unwrap();
    web.submit_frame(&batch.into_bytes()).unwrap();
}

#[test]
fn typed_flex_basis_hashes_emits_and_resets_without_raw_props() {
    for key in ["flex-basis", "flexBasis", "flex_basis"] {
        assert!(!is_raw_style_prop(key), "{key} must not be a raw style prop");
    }

    let mut web = WebBackend::new();
    let bases = [
        (2, Dimension::Points(280.0), "280px"),
        (3, Dimension::Points(110.0), "110px"),
        // `0%` and the length `0` differ for indefinite containers: distinct rules.
        (4, Dimension::Percent(0.0), "0%"),
        (5, Dimension::Points(0.0), "0"),
    ];
    submit(&mut web, 1, |s| {
        for &(id, basis, _) in &bases {
            s.create_node(id, ComponentKind::View, None);
            s.set_layout_ext(id, Some(&basis_ext(basis)));
        }
        s.create_node(6, ComponentKind::View, None);
        s.set_layout_ext(6, Some(&basis_ext(Dimension::Auto)));
        s.set_children(ROOT_NODE_ID, &[2, 3, 4, 5, 6]);
    });

    let ssr = web.render_ssr();
    for (_, _, css) in bases {
        assert_eq!(
            ssr.style_css.matches(&format!("flex-basis:{css};")).count(),
            1,
            "each distinct basis needs exactly one structural rule: {}",
            ssr.style_css
        );
    }
    // The base reset owns the only `flex:` shorthand, so a node's basis longhand is never reset.
    assert!(NVE_BASE_RESET_CSS.contains("flex:0 1 auto"));
    assert!(!ssr.style_css.contains("flex:"), "{}", ssr.style_css);
    assert_eq!(ssr.style_css.matches("flex-basis:").count(), 4, "auto emits nothing");
    assert!(!ssr.html.contains("flex-basis="), "{}", ssr.html);
    assert!(!ssr.html.contains("flexBasis="), "{}", ssr.html);

    // Clearing the `LayoutExt` (inline `flex` removed) drops the basis from the node's class.
    let before = web.render_ssr().html;
    submit(&mut web, 2, |s| {
        s.set_layout_ext(2, None);
        s.set_layout_ext(3, Some(&basis_ext(Dimension::Points(280.0))));
    });
    let after = web.render_ssr();
    assert_ne!(before, after.html, "node classes change with the basis");
    let class_of = |html: &str, id: u32| -> String {
        let at = html.find(&format!("data-nve-id=\"{id}\"")).unwrap();
        let tag_start = html[..at].rfind('<').unwrap();
        let tag = &html[tag_start..at + html[at..].find('>').unwrap()];
        let cls = tag.split("class=\"").nth(1).unwrap();
        cls[..cls.find('"').unwrap()].to_owned()
    };
    assert_eq!(class_of(&after.html, 2), class_of(&after.html, 6), "cleared == auto");
    assert_eq!(class_of(&after.html, 3), class_of(&before, 2), "same basis, same class");
}
