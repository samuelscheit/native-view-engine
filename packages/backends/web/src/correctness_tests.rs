//! Regression tests for mirror / live-DOM correctness on hostile or corrupt CommandBuffers:
//! HTML escaping, structural guards (cycles, repeats, moves, the root container, deep trees),
//! CSS-only raw props, and attribute parity between `render_html`, SSR and the wasm DOM path.

use nve_abi::{CommandBufferBuilder, ComponentKind as K, RawPropValue as V, ROOT_NODE_ID as ROOT};
use nve_components::canvas_ir::DrawListBuilder;
use nve_engine::RenderBackend;

use crate::WebBackend;

fn apply(web: &mut WebBackend, build: impl FnOnce(&mut CommandBufferBuilder)) {
    let mut b = CommandBufferBuilder::new(1, 1);
    build(&mut b);
    b.commit(true);
    web.submit_frame(b.finish()).expect("frame applies");
}

fn s(v: &str) -> V {
    V::String(v.into())
}

fn count(haystack: &str, needle: &str) -> usize {
    haystack.matches(needle).count()
}

#[test]
fn render_html_and_ssr_escape_text_and_every_attribute_value() {
    let mut web = WebBackend::new();
    let evil = "</div><script>alert(\"x\")</script>&amp;'";
    apply(&mut web, |b| {
        b.create_node(2, K::Text, None);
        b.set_text(2, evil);
        b.set_raw_prop(2, "accessibilityLabel", &s(evil));
        b.create_node(3, K::Image, None);
        b.set_raw_prop(3, "src", &s("a.png?x=\"1\"&y=<2>"));
        b.set_raw_prop(3, "alt", &s("\"alt\" & <alt>"));
        b.set_children(ROOT, &[2, 3]);
    });
    for html in [web.render_html(), web.render_ssr().html] {
        assert!(!html.contains("<script>"), "unescaped markup in {html}");
        assert!(html.contains("&lt;/div&gt;&lt;script&gt;alert(\"x\")&lt;/script&gt;&amp;amp;'</"));
        assert!(html.contains("aria-label=\"&lt;/div&gt;&lt;script&gt;alert(&quot;x&quot;)"));
        assert!(html.contains("src=\"a.png?x=&quot;1&quot;&amp;y=&lt;2&gt;\""));
        assert!(html.contains("alt=\"&quot;alt&quot; &amp; &lt;alt&gt;\""));
    }
}

#[test]
fn raw_css_values_cannot_inject_rules_or_close_the_ssr_style_element() {
    let hostile = [
        "red}</style><script>alert(1)</script>",
        "red;} .nve{display:none",
        "0 0 1px red; background:url(x)",
        "700/*",
        "\"inherit\"",
        "center\\7d",
        "hidden!important",
    ];
    let mut web = WebBackend::new();
    apply(&mut web, |b| {
        for (i, value) in hostile.iter().enumerate() {
            let id = 2 + i as u32;
            b.create_node(id, K::View, None);
            for key in ["box-shadow", "font-weight", "text-align", "overflow", "flex-wrap"] {
                b.set_raw_prop(id, key, &s(value));
            }
        }
        b.create_node(20, K::View, None);
        b.set_raw_prop(20, "box-shadow", &s(" 0 1px 2px rgba(0, 0, 0, .25), inset 0 0 0 1px #fff "));
        b.set_raw_prop(20, "font-weight", &V::I64(600));
        b.set_children(ROOT, &(2..2 + hostile.len() as u32).chain([20]).collect::<Vec<_>>());
    });
    let doc = web.render_ssr_document(&crate::SsrDocumentOptions::default());
    let mirror = web.render_html();
    for out in [&doc, &mirror] {
        for needle in ["<script", ".nve{display:none", "url(", "/*", "!important", "\\", "\"inherit\""] {
            assert!(!out.contains(needle), "{needle} leaked into {out}");
        }
        assert!(out.contains("box-shadow:0 1px 2px rgba(0, 0, 0, .25), inset 0 0 0 1px #fff;"), "{out}");
        assert!(out.contains("font-weight:600;"), "{out}");
    }
    assert_eq!(doc.matches("</style>").count(), 2, "only the base and SSR style elements close: {doc}");
    // A later valid value replaces a rejected one; a rejected one clears a valid one.
    apply(&mut web, |b| b.set_raw_prop(2, "overflow", &s("scroll")));
    assert!(web.render_html().contains("overflow:scroll;"));
    apply(&mut web, |b| b.set_raw_prop(2, "overflow", &s("scroll;}")));
    assert!(!web.render_html().contains("overflow:"));
}

#[test]
fn cycles_repeats_and_moves_render_each_node_once_like_the_dom() {
    let mut web = WebBackend::new();
    apply(&mut web, |b| {
        for id in 2..=5 {
            b.create_node(id, K::View, None);
        }
        b.set_children(ROOT, &[2, 3]);
        b.set_children(2, &[4, 5]);
    });
    // Repeats, unknown ids, the root and the parent itself are skipped; 4 and 5 move to 3.
    apply(&mut web, |b| b.set_children(3, &[5, 5, 4, 99, ROOT, 3, 4]));
    // 3 is an ancestor of 5 (cycle, rejected); 2 is not an ancestor of 4 (moves under it).
    apply(&mut web, |b| {
        b.set_children(5, &[3]);
        b.set_children(4, &[2]);
    });
    let html = web.render_html();
    for id in 1..=5 {
        assert_eq!(count(&html, &format!("data-nve-id=\"{id}\"")), 1, "node {id} once in {html}");
    }
    let pos = |id: u32| html.find(&format!("data-nve-id=\"{id}\"")).unwrap();
    assert!(pos(3) < pos(5) && pos(5) < pos(4) && pos(4) < pos(2), "3 > [5, 4 > [2]]: {html}");
    assert_eq!(web.render_ssr().html.matches("data-nve-id=").count(), 5);
}

#[test]
fn deep_trees_serialize_without_recursion() {
    const DEPTH: u32 = 200_000;
    let mut web = WebBackend::new();
    apply(&mut web, |b| {
        b.create_node(2, K::View, None);
        b.set_children(ROOT, &[2]);
        for id in 3..DEPTH {
            b.create_node(id, K::View, None);
            b.set_children(id - 1, &[id]);
        }
    });
    let html = web.render_html();
    assert_eq!(count(&html, "</div>"), DEPTH as usize - 1);
    let ssr = web.render_ssr();
    assert_eq!(count(&ssr.html, "</div>"), DEPTH as usize - 1);
}

#[test]
fn css_only_raw_props_become_styles_never_html_attributes() {
    let mut web = WebBackend::new();
    apply(&mut web, |b| {
        b.create_node(2, K::View, None);
        b.set_raw_prop(2, "flex-wrap", &s("wrap"));
        b.create_node(3, K::View, None);
        b.set_raw_prop(3, "flexWrap", &s("wrap-reverse"));
        b.create_node(4, K::Text, None);
        b.set_raw_prop(4, "font-weight", &s("700"));
        b.set_raw_prop(4, "text-align", &s("center"));
        b.set_raw_prop(4, "box-shadow", &s("0 1px 2px red"));
        b.set_raw_prop(4, "overflow", &s("hidden"));
        b.set_children(ROOT, &[2, 3, 4]);
    });
    let html = web.render_html();
    assert!(html.contains("flex-wrap:wrap;"), "{html}");
    assert!(html.contains("flex-wrap:wrap-reverse;"), "{html}");
    for decl in ["font-weight:700;", "text-align:center;", "box-shadow:0 1px 2px red;", "overflow:hidden;"] {
        assert!(html.contains(decl), "{decl} missing in {html}");
    }
    let ssr = web.render_ssr();
    assert!(ssr.style_css.contains("flex-wrap:wrap;") || ssr.html.contains("flex-wrap:wrap;"));
    for attr in [" flex-wrap=", " flexWrap=", " font-weight=", " text-align=", " box-shadow=", " overflow="] {
        assert!(!html.contains(attr) && !ssr.html.contains(attr), "{attr} leaked as an attribute");
    }
}

#[test]
fn the_root_container_is_never_replaced_removed_retagged_or_reparented() {
    let mut web = WebBackend::new();
    apply(&mut web, |b| {
        b.create_node(2, K::View, None);
        b.set_text(2, "child");
        b.set_children(ROOT, &[2]);
    });
    let before = web.render_html();
    apply(&mut web, |b| {
        b.create_node(ROOT, K::Text, None);
        b.set_raw_prop(ROOT, "accessibilityRole", &s("navigation"));
        b.set_raw_prop(ROOT, "href", &s("/x"));
        b.set_children(2, &[ROOT]);
        b.remove_node(ROOT);
    });
    // The root keeps its tag (no `<nav>`/`<a>` retag) and its children; `role` is an ordinary
    // attribute on the container, written the same way by the live DOM path.
    let expected = before.replacen("<div data-nve-id=\"1\"", "<div data-nve-id=\"1\" role=\"navigation\"", 1);
    assert_eq!(web.render_html(), expected);
    assert!(web.node(ROOT).is_some_and(|n| n.children == [2] && n.kind == K::Root));
    // A detached parent has no root ancestor: the root is still never its child.
    apply(&mut web, |b| {
        b.create_node(3, K::View, None);
        b.set_children(3, &[ROOT]);
        b.set_children(ROOT, &[2, 3]);
    });
    assert!(web.node(3).is_some_and(|n| n.children.is_empty()));
    assert!(web.render_html().starts_with("<div data-nve-id=\"1\""));
}

#[test]
fn text_shows_only_while_a_node_has_no_children() {
    let mut web = WebBackend::new();
    apply(&mut web, |b| {
        b.create_node(2, K::Text, None);
        b.set_text(2, "own text");
        b.create_node(3, K::Text, None);
        b.set_text(3, "child");
        b.set_children(ROOT, &[2]);
        b.set_children(2, &[3]);
    });
    for html in [web.render_html(), web.render_ssr().html] {
        assert!(!html.contains("own text") && html.contains("child"), "{html}");
    }
    apply(&mut web, |b| b.set_children(2, &[]));
    assert!(web.render_html().contains("own text"));
}

#[test]
fn draw_ops_attribute_and_input_type_match_between_mirror_ssr_and_dom() {
    let mut web = WebBackend::new();
    let mut list = DrawListBuilder::new();
    list.clear(nve_abi::ColorRgba::BLACK);
    apply(&mut web, |b| {
        b.create_node(2, K::SkiaCanvas, None);
        b.set_raw_prop(2, "drawList", &V::Bytes(list.finish()));
        b.create_node(3, K::SkiaCanvas, None);
        b.set_raw_prop(3, "drawList", &s("nve-draw1;b64,@@@@"));
        b.create_node(4, K::TextInput, None);
        b.set_raw_prop(4, "secureTextEntry", &V::Bool(false));
        b.create_node(5, K::TextInput, None);
        b.set_raw_prop(5, "secureTextEntry", &V::Bool(true));
        b.set_children(ROOT, &[2, 3, 4, 5]);
    });
    for html in [web.render_html(), web.render_ssr().html] {
        assert_eq!(count(&html, "data-nve-draw-ops=\"1\""), 1, "{html}");
        assert_eq!(count(&html, "data-nve-draw-ops"), 1, "an empty draw list has no attribute: {html}");
        assert_eq!(count(&html, " type="), 1, "only the secure input has a type: {html}");
        assert!(html.contains("type=\"password\""));
    }
}
