//! Edge-case parity & regression tests for [`super::CompositorScene`] (part 1):
//! - `StateRules` (`:hover`, `:active`, `:focus`, `:disabled` including `display: none` toggles
//!   on `PaintOnly` commits & in-place `StateRules` resource redefinition)
//! - `display: none` ancestor child paint (no over-escalation to `sync_full_tree`)
//! - Nested transforms + group opacity edge values (`0.0`, `1.0`, `0.5`, `NaN`, `Inf`) + clip+radius
//!   under transform + `tx.detach::<Transform>()`
//! - Cross-backend Software (`push_mul`) vs CPU Skia (`save_layer_alpha_f`) group opacity semantics
//! - `z_index` on containers with children + 0-churn in-place `CompositeOnly` reorder.

use nve_abi::{ColorRgba, SurfaceId};
use nve_engine::{StoreBackend, Viewport};
use nve_store::wire::{Op, WireDecoder, MAGIC, VERSION};
use nve_store::{ChangeSet, FlatResource, Length, NodeId, NodeStore, ResourceId, Rgba};
use nve_tables::interaction::{FocusModality, InteractionState, StateStyleRule};
use nve_tables::tables::{
    Display, FlexDirection, Interaction, Layout, Overflow, Position, StateRules, Transform, Visual,
};
use nve_tables::{RowPatch, StylePatchExt};

use super::oracle_tests::assert_scene_matches_fresh_display_list;
use super::SceneDirtyKind;
#[cfg(feature = "skia")]
use crate::raster::PixelBuffer;
use crate::SkiaBackend;

pub(super) fn build_redefine_resource_frame<R: FlatResource>(rid: ResourceId, value: &R) -> Vec<u8> {
    let mut payload_words = Vec::new();
    value.write_words(&mut payload_words);
    let byte_len = (payload_words.len() * 4) as u32;
    let mut body = Vec::with_capacity(24 + payload_words.len() * 4);
    body.extend_from_slice(&rid.0.to_le_bytes());
    body.extend_from_slice(&(R::KIND.0 as u32).to_le_bytes());
    body.extend_from_slice(&((R::KIND.0 >> 32) as u32).to_le_bytes());
    body.extend_from_slice(&1u32.to_le_bytes()); // flags: ephemeral = true
    body.extend_from_slice(&0u32.to_le_bytes()); // child_count = 0
    body.extend_from_slice(&byte_len.to_le_bytes());
    for w in &payload_words {
        body.extend_from_slice(&w.to_le_bytes());
    }

    let mut frame = Vec::with_capacity(32 + body.len());
    frame.extend_from_slice(&MAGIC.to_le_bytes());
    frame.extend_from_slice(&VERSION.to_le_bytes());
    frame.extend_from_slice(&0u16.to_le_bytes());
    frame.extend_from_slice(&1u64.to_le_bytes());
    frame.extend_from_slice(&1u32.to_le_bytes());
    let total_words = (6 + 2 + body.len() / 4) as u32;
    frame.extend_from_slice(&total_words.to_le_bytes());
    let head = (Op::DefineResources as u32) | (1u32 << 16);
    frame.extend_from_slice(&head.to_le_bytes());
    frame.extend_from_slice(&((body.len() / 4) as u32).to_le_bytes());
    frame.extend_from_slice(&body);
    frame
}

#[test]
fn state_rules_hover_active_focus_disabled_display_none_and_resource_redef() {
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let mut backend = SkiaBackend::new();
    backend.set_viewport(
        SurfaceId::ROOT,
        Viewport {
            width: 160.0,
            height: 120.0,
            scale_factor: 1.0,
        },
    );

    let rules_v1 = StateRules::new(vec![
        StateStyleRule::new(
            InteractionState::HOVER,
            RowPatch::new().bg(ColorRgba::rgb(20, 180, 120)),
        ),
        StateStyleRule::new(
            InteractionState::ACTIVE,
            RowPatch::new().bg(ColorRgba::rgb(220, 60, 60)),
        ),
        StateStyleRule::new(
            InteractionState::FOCUS,
            RowPatch::new()
                .border_color(ColorRgba::rgb(250, 204, 21))
                .border_width(3.0),
        ),
        StateStyleRule::new(
            InteractionState::DISABLED,
            RowPatch::new().display(Display::None),
        ),
    ]);

    let (btn, btn_child) = {
        let mut tx = store.tx();
        tx.edit::<Layout>(NodeId::ROOT)
            .width(Length::points(160.0))
            .height(Length::points(120.0));
        let btn = tx.create();
        tx.edit::<Layout>(btn)
            .width(Length::points(100.0))
            .height(Length::points(50.0));
        tx.edit::<Visual>(btn).background(Rgba::rgb(50, 80, 180));
        tx.edit::<Interaction>(btn)
            .focusable(true)
            .state_rules_ephemeral(rules_v1);
        let child = tx.create();
        tx.edit::<Layout>(child)
            .width(Length::points(60.0))
            .height(Length::points(24.0));
        tx.edit::<Visual>(child).background(Rgba::rgb(240, 240, 240));
        tx.set_children(btn, &[child]);
        tx.set_children(NodeId::ROOT, &[btn]);
        tx.commit_into(&mut changes);
        (btn, child)
    };
    backend.commit(&store, &changes).expect("init");
    assert_scene_matches_fresh_display_list(&mut backend, &store, 160, 120, "state_rules init");

    // 1. :hover, :active, :focus transitions via InteractionTracker
    assert!(backend.pointer_move(&store, 20.0, 20.0));
    assert_scene_matches_fresh_display_list(&mut backend, &store, 160, 120, "state_rules hover");
    assert!(backend.pointer_down(&store, 20.0, 20.0));
    assert_scene_matches_fresh_display_list(&mut backend, &store, 160, 120, "state_rules active");
    assert!(backend.pointer_up(&store, 20.0, 20.0));
    assert!(backend.set_interaction_focus(&store, Some(btn), FocusModality::Keyboard));
    assert_scene_matches_fresh_display_list(&mut backend, &store, 160, 120, "state_rules focus");

    // 2. Interaction::disabled(true) routes to PaintOnly -> MUST detect display:none!
    {
        let mut tx = store.tx();
        tx.edit::<Interaction>(btn).disabled(true);
        tx.commit_into(&mut changes);
    }
    backend.commit(&store, &changes).expect("disable -> display:none");
    assert!(
        backend.scene().chunk_for_node(btn).is_none()
            && backend.scene().chunk_for_node(btn_child).is_none(),
        "disabled display:none must retire btn and its child subtree"
    );
    assert_scene_matches_fresh_display_list(&mut backend, &store, 160, 120, "disabled display:none");

    // 3. In-place StateRules resource redefinition while node is display:none -> removes display:none!
    let rules_rid = store.row::<Interaction>(btn).state_rules();
    let rules_v2 = StateRules::new(vec![StateStyleRule::new(
        InteractionState::DISABLED,
        RowPatch::new().bg(ColorRgba::rgb(120, 120, 120)),
    )]);
    let frame = build_redefine_resource_frame(rules_rid, &rules_v2);
    let mut dec = WireDecoder::default();
    {
        let mut tx = store.tx();
        dec.apply(&mut tx, &frame).expect("redefine StateRules");
        tx.commit_into(&mut changes);
    }
    backend.commit(&store, &changes).expect("redefined StateRules commit");
    assert!(
        backend.scene().chunk_for_node(btn).is_some()
            && backend.scene().chunk_for_node(btn_child).is_some(),
        "redefining StateRules resource on currently display:none node must restore chunks"
    );
    assert_scene_matches_fresh_display_list(&mut backend, &store, 160, 120, "StateRules res redef");

    // 4. Toggle back to disabled(false)
    {
        let mut tx = store.tx();
        tx.edit::<Interaction>(btn).disabled(false);
        tx.commit_into(&mut changes);
    }
    backend.commit(&store, &changes).expect("reenable");
    assert_scene_matches_fresh_display_list(&mut backend, &store, 160, 120, "reenabled");
}

#[test]
fn display_none_ancestor_child_paint_does_not_escalate_to_full_tree() {
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let mut backend = SkiaBackend::new();
    backend.set_viewport(
        SurfaceId::ROOT,
        Viewport {
            width: 120.0,
            height: 80.0,
            scale_factor: 1.0,
        },
    );

    let child = {
        let mut tx = store.tx();
        tx.edit::<Layout>(NodeId::ROOT)
            .width(Length::points(120.0))
            .height(Length::points(80.0));
        let hidden_parent = tx.create();
        tx.edit::<Layout>(hidden_parent)
            .display(Display::None)
            .width(Length::points(100.0))
            .height(Length::points(60.0));
        let child = tx.create();
        tx.edit::<Layout>(child)
            .width(Length::points(50.0))
            .height(Length::points(30.0));
        tx.edit::<Visual>(child).background(Rgba::rgb(10, 200, 30));
        tx.set_children(hidden_parent, &[child]);
        tx.set_children(NodeId::ROOT, &[hidden_parent]);
        tx.commit_into(&mut changes);
        child
    };
    backend.commit(&store, &changes).expect("init");
    let syncs_before = backend.scene().stats().layout_tree_syncs;

    {
        let mut tx = store.tx();
        tx.edit::<Visual>(child).background(Rgba::rgb(220, 40, 40));
        tx.commit_into(&mut changes);
    }
    backend.commit(&store, &changes).expect("paint hidden child");
    assert_eq!(
        backend.scene().stats().layout_tree_syncs,
        syncs_before,
        "editing a child under a display:none ancestor must NOT escalate to sync_full_tree"
    );
    assert_scene_matches_fresh_display_list(&mut backend, &store, 120, 80, "display:none parent");
}

#[test]
fn nested_transforms_opacity_edge_values_clip_radius_and_detach_transform() {
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let mut backend = SkiaBackend::new();
    backend.set_viewport(
        SurfaceId::ROOT,
        Viewport {
            width: 200.0,
            height: 160.0,
            scale_factor: 1.0,
        },
    );

    let (outer, inner, edge_card) = {
        let mut tx = store.tx();
        tx.edit::<Layout>(NodeId::ROOT)
            .width(Length::points(200.0))
            .height(Length::points(160.0))
            .flex_direction(FlexDirection::Row);
        let outer = tx.create();
        tx.edit::<Layout>(outer)
            .width(Length::points(90.0))
            .height(Length::points(90.0));
        tx.edit::<Visual>(outer)
            .background(Rgba::rgb(30, 64, 175))
            .border_radius(14.0)
            .overflow_x(Overflow::Hidden)
            .overflow_y(Overflow::Hidden)
            .opacity(0.6);
        tx.edit::<Transform>(outer).translate_x(12.0).rotate_deg(8.0);

        let inner = tx.create();
        tx.edit::<Layout>(inner)
            .width(Length::points(70.0))
            .height(Length::points(70.0));
        tx.edit::<Visual>(inner)
            .background(Rgba::rgb(234, 88, 12))
            .border_radius(8.0)
            .opacity(0.75);
        tx.edit::<Transform>(inner).translate_y(10.0).scale_x(0.85);
        tx.set_children(outer, &[inner]);

        let edge_card = tx.create();
        tx.edit::<Layout>(edge_card)
            .width(Length::points(60.0))
            .height(Length::points(60.0));
        tx.edit::<Visual>(edge_card)
            .background(Rgba::rgb(16, 185, 129))
            .opacity(0.0);

        tx.set_children(NodeId::ROOT, &[outer, edge_card]);
        tx.commit_into(&mut changes);
        (outer, inner, edge_card)
    };
    backend.commit(&store, &changes).expect("init");
    assert_scene_matches_fresh_display_list(&mut backend, &store, 200, 160, "nested tf + op=0.0");

    // Exercise opacity edge values: 1.0, NaN, Inf, 0.5.
    for (label, op) in [
        ("op=1.0", 1.0f32),
        ("op=NaN", f32::NAN),
        ("op=Inf", f32::INFINITY),
        ("op=0.5", 0.5),
    ] {
        let mut tx = store.tx();
        tx.edit::<Visual>(edge_card).opacity(op);
        tx.commit_into(&mut changes);
        backend.commit(&store, &changes).expect(label);
        assert_scene_matches_fresh_display_list(&mut backend, &store, 200, 160, label);
    }

    // Exercise tx.detach::<Transform>()
    {
        let mut tx = store.tx();
        tx.detach::<Transform>(outer);
        tx.detach::<Transform>(inner);
        tx.commit_into(&mut changes);
    }
    backend.commit(&store, &changes).expect("detach transform");
    assert_scene_matches_fresh_display_list(&mut backend, &store, 200, 160, "detach transform");
}

#[cfg(feature = "skia")]
#[test]
fn cross_backend_group_opacity_semantics_non_overlapping_and_overlapping() {
    use crate::gpu::{CpuSkiaSurface, GpuSurfaceBackend};

    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let mut backend = SkiaBackend::new();
    backend.set_viewport(
        SurfaceId::ROOT,
        Viewport {
            width: 120.0,
            height: 80.0,
            scale_factor: 1.0,
        },
    );

    {
        let mut tx = store.tx();
        tx.edit::<Layout>(NodeId::ROOT)
            .width(Length::points(120.0))
            .height(Length::points(80.0))
            .flex_direction(FlexDirection::Row);

        // 1. Non-overlapping translucent leaf (Software push_mul == Skia save_layer_alpha_f)
        let single = tx.create();
        tx.edit::<Layout>(single)
            .width(Length::points(40.0))
            .height(Length::points(40.0));
        tx.edit::<Visual>(single)
            .background(Rgba::rgb(200, 100, 50))
            .opacity(0.5);

        // 2. Overlapping opaque parent (red) + opaque child (green) inside 0.5 opacity group:
        //    Skia save_layer_alpha_f composites green over red offscreen -> pure green @ 0.5 alpha.
        let group = tx.create();
        tx.edit::<Layout>(group)
            .width(Length::points(40.0))
            .height(Length::points(40.0));
        tx.edit::<Visual>(group)
            .background(Rgba::rgb(255, 0, 0))
            .opacity(0.5);
        let child = tx.create();
        tx.edit::<Layout>(child)
            .width(Length::points(40.0))
            .height(Length::points(40.0));
        tx.edit::<Visual>(child).background(Rgba::rgb(0, 255, 0));
        tx.set_children(group, &[child]);

        // 3. Empty translucent container (tests deferred save_layer_alpha_f elision in Skia sink).
        let empty_group = tx.create();
        tx.edit::<Layout>(empty_group)
            .width(Length::points(40.0))
            .height(Length::points(40.0));
        tx.edit::<Visual>(empty_group).opacity(0.35);

        tx.set_children(NodeId::ROOT, &[single, group, empty_group]);
        tx.commit_into(&mut changes);
    }
    backend.commit(&store, &changes).expect("init");

    let mut soft_buf = PixelBuffer::new(120, 80);
    backend.rasterize(&mut soft_buf, ColorRgba::BLACK);

    let mut skia_surf = CpuSkiaSurface::new(120, 80).expect("cpu skia");
    skia_surf
        .render_scene_cached_with_plugins(backend.scene(), 1.0, ColorRgba::BLACK, None)
        .expect("skia render");
    let mut skia_buf = PixelBuffer::new(120, 80);
    skia_surf.copy_to_pixel_buffer(&mut skia_buf).expect("readback");

    // Non-overlapping single rect matches across Software & Skia within ±2/255 rounding.
    let (sr, sg, sb) = soft_buf.get_rgb(20, 20).expect("soft single");
    let (kr, kg, kb) = skia_buf.get_rgb(20, 20).expect("skia single");
    assert!(
        sr.abs_diff(kr) <= 2 && sg.abs_diff(kg) <= 2 && sb.abs_diff(kb) <= 2,
        "non-overlapping opacity must match across Software ({sr},{sg},{sb}) & Skia ({kr},{kg},{kb})"
    );

    // Overlapping group: Skia isolated offscreen compositing has near-zero red bleed.
    let (gr, gg, _gb) = skia_buf.get_rgb(60, 20).expect("skia group");
    assert!(
        gr <= 2 && gg >= 120,
        "Skia save_layer_alpha_f must isolate opaque green child over red parent (got {gr},{gg})"
    );
    // Empty translucent container leaves clear black background untouched.
    assert_eq!(skia_buf.get_rgb(100, 20), Some((0, 0, 0)));
}

#[test]
fn z_index_on_nodes_with_children_and_inplace_composite_reorder() {
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let mut backend = SkiaBackend::new();
    backend.set_viewport(
        SurfaceId::ROOT,
        Viewport {
            width: 120.0,
            height: 120.0,
            scale_factor: 1.0,
        },
    );

    let (box_a, box_b) = {
        let mut tx = store.tx();
        tx.edit::<Layout>(NodeId::ROOT)
            .width(Length::points(120.0))
            .height(Length::points(120.0));
        let box_a = tx.create();
        tx.edit::<Layout>(box_a)
            .position(Position::Absolute)
            .inset_left(Length::points(10.0))
            .inset_top(Length::points(10.0))
            .width(Length::points(70.0))
            .height(Length::points(70.0));
        tx.edit::<Visual>(box_a).background(Rgba::rgb(220, 38, 38)).z_index(1);
        let child_a = tx.create();
        tx.edit::<Layout>(child_a)
            .width(Length::points(40.0))
            .height(Length::points(40.0));
        tx.edit::<Visual>(child_a).background(Rgba::rgb(254, 202, 202));
        tx.set_children(box_a, &[child_a]);

        let box_b = tx.create();
        tx.edit::<Layout>(box_b)
            .position(Position::Absolute)
            .inset_left(Length::points(25.0))
            .inset_top(Length::points(25.0))
            .width(Length::points(70.0))
            .height(Length::points(70.0));
        tx.edit::<Visual>(box_b).background(Rgba::rgb(37, 99, 235)).z_index(2);
        let child_b = tx.create();
        tx.edit::<Layout>(child_b)
            .width(Length::points(40.0))
            .height(Length::points(40.0));
        tx.edit::<Visual>(child_b).background(Rgba::rgb(191, 219, 254));
        tx.set_children(box_b, &[child_b]);

        tx.set_children(NodeId::ROOT, &[box_a, box_b]);
        tx.commit_into(&mut changes);
        (box_a, box_b)
    };
    backend.commit(&store, &changes).expect("init");
    assert_scene_matches_fresh_display_list(&mut backend, &store, 120, 120, "z_index init");

    let emitted_before = backend.scene().stats().chunks_emitted;
    let dead_before = backend.scene().arenas.dead_children;
    {
        let mut tx = store.tx();
        tx.edit::<Visual>(box_a).z_index(10);
        tx.edit::<Visual>(box_b).z_index(1);
        tx.commit_into(&mut changes);
    }
    backend.commit(&store, &changes).expect("z_index swap");
    assert_eq!(
        backend.scene().stats().last_dirty_kind,
        Some(SceneDirtyKind::CompositeOnly)
    );
    assert_eq!(backend.scene().stats().chunks_emitted, emitted_before);
    assert_eq!(backend.scene().arenas.dead_children, dead_before);
    assert_scene_matches_fresh_display_list(&mut backend, &store, 120, 120, "z_index swapped");
}
