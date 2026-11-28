//! `rsx!` structure: control-flow heads, template paths, native tags and text ownership.

use super::super::patch::TypedRsxPatch;
use super::{assert_tier2_reason, diff_rsx_source, expect_patch, typed_patch};

fn component(body: &str) -> String {
    format!(
        "fn c(state: State, items: Vec<Item>, x: u8, on: bool) -> Element {{\n    rsx! {{\n        view {{\n            test_id: \"root\",\n            {body}\n        }}\n    }}\n}}\n"
    )
}

fn patch(old: &str, new: &str) -> TypedRsxPatch {
    typed_patch(old, new)
        .unwrap_or_else(|e| panic!("{e}"))
        .unwrap_or_else(|| panic!("expected a typed patch"))
}

#[test]
fn pattern_and_head_literals_are_rust_not_rsx_text() {
    let heads = [
        "if let State { label: \"ready\", .. } = state { view { \"ok\" } }",
        "for Item { tag: \"ready\", .. } in items { view { \"ok\" } }",
        "if { let s = \"ready\"; s == x } { view { \"ok\" } }",
        "if match x { 1 => \"ready\" == \"a\", _ => false } { view { \"ok\" } }",
        "if let Some(a) = x && let P { q: \"ready\" } = state { view { \"ok\" } }",
        "if (S { a: \"ready\" }).ok() { view { \"ok\" } }",
        "if on { view { \"no\" } } else if let S { a: \"ready\" } = state { view { \"ok\" } }",
    ];
    for head in heads {
        let base = component(head);
        assert_tier2_reason(
            &base,
            &base.replacen("\"ready\"", "\"steady\"", 1),
            "structural or logic edit",
        );
        let body_edit = base.replace("\"ok\"", "\"fine\"");
        let msg = expect_patch(diff_rsx_source("c.rs", &base, &body_edit, 2));
        assert_eq!(
            msg.text_replacements,
            vec![("ok".into(), "fine".into())],
            "{head}"
        );
        assert_eq!(msg.target_test_id.as_deref(), Some("root"), "{head}");
    }
}

#[test]
fn control_flow_bodies_are_templates_and_take_one_parent_slot() {
    let base = component(
        "text { \"a\" } if on { text { \"b\" } } else { text { \"e\" } } for i in 0..2 { text { \"c\" } } rsx! { text { \"n\" } } { let v = 1; } text { \"d\" }",
    );
    let edited = ["a", "b", "e", "c", "n", "d"]
        .iter()
        .fold(base.clone(), |src, t| {
            src.replace(&format!("\"{t}\""), &format!("\"{t}!\""))
        });
    let patch = patch(&base, &edited);
    let coords: Vec<_> = patch
        .text_changes
        .iter()
        .map(|c| {
            (
                c.old.as_str(),
                c.target.template.ordinal,
                c.target.node_path.clone(),
            )
        })
        .collect();
    assert_eq!(
        coords,
        vec![
            ("a", 0, vec![0, 0, 0]),
            ("b", 1, vec![0, 0]),
            ("e", 2, vec![0, 0]),
            ("c", 3, vec![0, 0]),
            ("n", 4, vec![0, 0]),
            // `text "a"`, the if/else, the for, the nested rsx! and the block: slot 5.
            ("d", 0, vec![0, 5, 0]),
        ]
    );
    assert!(
        patch
            .text_changes
            .iter()
            .all(|c| c.target.test_id.as_deref() == Some("root"))
    );
}

#[test]
fn only_native_tags_have_patchable_attributes_and_static_test_ids() {
    for element in [
        "my_card { padding: 8.0 }",
        "ui::view { test_id: \"v\", padding: 8.0 }",
        "Card { test_id: \"c\", padding: 8.0 }",
    ] {
        let base = component(element);
        assert_tier2_reason(
            &base,
            &base.replace("8.0", "9.0"),
            "structural or logic edit",
        );
    }
    // A component's `test_id` is a prop: text inside it belongs to the nearest native ancestor.
    let base = component("Card { test_id: \"card\", \"inside\" } view { view { \"deep\" } }");
    let edited = base
        .replace("\"inside\"", "\"in\"")
        .replace("\"deep\"", "\"deeper\"");
    let patch = patch(&base, &edited);
    let owners: Vec<_> = patch
        .text_changes
        .iter()
        .map(|c| c.target.test_id.as_deref())
        .collect();
    assert_eq!(owners, vec![Some("root"), Some("root")]);
    // Every native tag of `nve_dioxus::dioxus_elements` is patchable.
    for tag in [
        "view",
        "div",
        "text",
        "span",
        "p",
        "a",
        "pressable",
        "button",
        "scrollview",
        "image",
        "textinput",
        "input",
    ] {
        let base = component(&format!("{tag} {{ test_id: \"t\", opacity: 1.0 }}"));
        let msg = expect_patch(diff_rsx_source(
            "c.rs",
            &base,
            &base.replace("1.0", "0.5"),
            2,
        ));
        assert_eq!(msg.target_test_id.as_deref(), Some("t"), "{tag}");
    }
}

#[test]
fn attribute_reordering_or_renaming_is_structural() {
    let base = component("view { test_id: \"v\", padding: 8.0, gap: 4.0 }");
    assert_tier2_reason(
        &base,
        &base.replace("padding: 8.0, gap: 4.0", "gap: 4.0, padding: 8.0"),
        "structural or logic edit",
    );
    assert_tier2_reason(
        &base,
        &base.replace("gap: 4.0", "margin: 4.0"),
        "structural",
    );
    assert_tier2_reason(
        &base,
        &base.replace("test_id: \"v\"", "test_id: \"w\""),
        "structural",
    );
}

#[test]
fn attribute_spread_before_children_does_not_shift_child_indices() {
    let base = component(
        "view { ..extra_attrs, text { \"First\" } view { padding: 4.0, text { \"Second\" } } }",
    );
    let edited = base
        .replace("\"First\"", "\"First!\"")
        .replace("padding: 4.0", "padding: 12.0");
    let p = patch(&base, &edited);
    assert_eq!(p.text_changes.len(), 1);
    assert_eq!(p.attr_changes.len(), 1);
    // `..extra_attrs,` must NOT consume a child slot: `text` is child 0 and inner `view` is child 1.
    assert_eq!(p.text_changes[0].target.node_path, vec![0, 0, 0, 0]);
    assert_eq!(p.attr_changes[0].target.node_path, vec![0, 0, 1]);
}
