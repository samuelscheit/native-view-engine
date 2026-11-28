//! End to end on a real `DioxusSurface`: lowered Tier-1 messages must leave the store exactly as
//! a fresh render of the edited source does, touching only the targeted node.

use nve_abi::{FrameId, SurfaceId};
use nve_tables::tables::{DebugId, Layout, TextContent, Typography, Visual};
use nve_dioxus::prelude::*;
use nve_store::{NodeId, NodeStore};

use super::super::DiffOutcome;
use super::diff_rsx_source;

/// Defines a component from an `rsx!` body and the source text the differ sees for it.
macro_rules! app_with_source {
    ($app:ident, $source:ident, { $($body:tt)* }) => {
        fn $app() -> Element {
            rsx! { $($body)* }
        }
        const $source: &str = concat!("fn app() -> Element { rsx! { ", stringify!($($body)*), " } }");
    };
}

app_with_source!(app_before, BEFORE, {
    view {
        view {
            test_id: "target",
            padding: 8.0, margin: 2.0, width: 10.0, min_height: 4.0, opacity: 0.5, z_index: 1,
            border_radius: 4.0, border_width: 1.0, flex_grow: 1.0, gap: 3.0, font_size: 14.0,
            text { "Label" }
        }
        view {
            test_id: "sibling",
            padding: 8.0, opacity: 0.5, font_size: 14.0,
            text { "Label" }
        }
    }
});

app_with_source!(app_after, AFTER, {
    view {
        view {
            test_id: "target",
            padding: -4.5, margin: -3.0, width: 33.3, min_height: 12.0, opacity: 1.25, z_index: -7,
            border_radius: 6.25, border_width: 2.0, flex_grow: 2.5, gap: 0.0, font_size: 0.25,
            text { "Relabeled" }
        }
        view {
            test_id: "sibling",
            padding: 8.0, opacity: 0.5, font_size: 14.0,
            text { "Label" }
        }
    }
});

fn find(store: &NodeStore, tid: &str) -> NodeId {
    let mut stack = vec![NodeId::ROOT];
    while let Some(id) = stack.pop() {
        if store
            .try_row::<DebugId>(id)
            .is_some_and(|d| d.test_id() == tid)
        {
            return id;
        }
        stack.extend_from_slice(store.children(id));
    }
    panic!("missing node with test_id={tid}");
}

/// Every style value the edited attributes write, plus the subtree's text.
fn snapshot(store: &NodeStore, tid: &str) -> String {
    let id = find(store, tid);
    let layout = store.try_row::<Layout>(id).map(|l| {
        format!(
            "w={:?} minh={:?} pt={:?} pl={:?} mt={:?} gap={:?} grow={:?}",
            l.width(),
            l.min_height(),
            l.padding_top(),
            l.padding_left(),
            l.margin_top(),
            l.row_gap(),
            l.flex_grow()
        )
    });
    let visual = store.try_row::<Visual>(id).map(|v| {
        format!(
            "bg={:?} op={:?} z={:?} r={:?} bw={:?}",
            v.background(),
            v.opacity(),
            v.z_index(),
            v.border_radius(),
            v.border_width()
        )
    });
    let font = store
        .try_row::<Typography>(id)
        .map(|t| (t.font_size(), t.explicit_mask()));
    let mut texts = Vec::new();
    let mut stack = vec![id];
    while let Some(n) = stack.pop() {
        if let Some(t) = store.try_row::<TextContent>(n) {
            texts.push(t.text().to_owned());
        }
        stack.extend_from_slice(store.children(n));
    }
    format!("{layout:?} {visual:?} {font:?} {texts:?}")
}

#[test]
fn lowered_patches_match_a_fresh_render_and_touch_only_the_target() {
    let DiffOutcome::Tier1Patch { patch, messages } = diff_rsx_source("app.rs", BEFORE, AFTER, 2)
    else {
        panic!("expected Tier1Patch");
    };
    assert_eq!(patch.attr_changes.len(), 11);
    assert_eq!(patch.text_changes.len(), 1);
    assert_eq!(messages.len(), 2);

    let mut patched = DioxusSurface::new(SurfaceId::ROOT, app_before);
    patched.reconcile(FrameId(1));
    let sibling_before = snapshot(patched.store(), "sibling");
    for msg in &messages {
        assert!(patched.apply_hot_patch(msg));
    }
    patched.reconcile(FrameId(2));

    let mut fresh = DioxusSurface::new(SurfaceId::ROOT, app_after);
    fresh.reconcile(FrameId(1));

    assert_eq!(
        snapshot(patched.store(), "target"),
        snapshot(fresh.store(), "target")
    );
    assert_eq!(snapshot(patched.store(), "sibling"), sibling_before);
    assert_eq!(snapshot(fresh.store(), "sibling"), sibling_before);
}

fn untagged_live_app() -> Element {
    rsx! {
        view {
            view {
                padding: 8.0,
                opacity: 1.0,
                text { "Untagged Live" }
            }
            for idx in 0..3 {
                view {
                    key: "{idx}",
                    padding: 4.0,
                    text { "Item {idx}" }
                }
            }
            view {
                padding: 6.0,
                text { "Untagged Sibling" }
            }
        }
    }
}

#[test]
fn end_to_end_rsx_diff_binary_row_patch_hits_untagged_element_and_for_loop() {
    let source = include_str!("surface_tests.rs");
    let edited = source
        .replacen("padding: 8.0,\n                opacity: 1.0,", "padding: 22.0,\n                opacity: 0.6,", 1)
        .replacen("padding: 4.0,", "padding: 16.0,", 1)
        .replacen("padding: 6.0,", "padding: 28.0,", 1)
        .replacen("\"Untagged Live\"", "\"Untagged Patched\"", 1);
    let DiffOutcome::Tier1Patch { messages, .. } =
        diff_rsx_source(file!(), source, &edited, 3)
    else {
        panic!("expected Tier1Patch for real untagged surface_tests.rs edit");
    };
    assert_eq!(messages.len(), 4, "1 text + 3 untagged element RowPatches");

    let mut surface = DioxusSurface::new(SurfaceId::ROOT, untagged_live_app);
    surface.reconcile(FrameId(1));
    for msg in &messages {
        assert!(surface.apply_hot_patch(msg));
    }
    surface.reconcile(FrameId(2));

    let root = surface.store().children(NodeId::ROOT)[0];
    let kids = surface.store().children(root);
    assert_eq!(kids.len(), 5, "1 header + 3 loop items + 1 footer");
    let header = kids[0];
    let footer = kids[4];
    assert_eq!(
        surface.store().try_row::<Layout>(header).unwrap().padding_top(),
        nve_store::Length::points(22.0)
    );
    assert!((surface.store().try_row::<Visual>(header).unwrap().opacity() - 0.6).abs() < 1e-5);
    for &loop_item in &kids[1..4] {
        assert_eq!(
            surface.store().try_row::<Layout>(loop_item).unwrap().padding_top(),
            nve_store::Length::points(16.0)
        );
    }
    assert_eq!(
        surface.store().try_row::<Layout>(footer).unwrap().padding_top(),
        nve_store::Length::points(28.0)
    );
    let header_text_el = surface.store().children(header)[0];
    let header_text_leaf = surface.store().children(header_text_el)[0];
    assert_eq!(
        surface.store().try_row::<TextContent>(header_text_leaf).unwrap().text(),
        "Untagged Patched"
    );
}
