//! Edge-case parity & regression tests for [`super::CompositorScene`] (part 2):
//! - `Image` + `ImageCache` invalidation + `SkiaTableRegistry` plugin parity
//! - Reparenting across different inherited typography & visibility + dynamic HiDPI scale changes
//! - In-place resource redefinition (gradient, box-shadow) + `SkiaFrameHook` composite & paint ticks
//! - Recycled `NodeId` generation ABA safety.

use nve_abi::{ColorRgba, SurfaceId};
use nve_engine::{StoreBackend, Viewport};
use nve_store::wire::WireDecoder;
use nve_store::{ChangeSet, Length, NodeId, NodeStore, NveTable, Rgba, TableSlot};
use nve_tables::paint::{BackgroundFill, GradientStop};
use nve_tables::snapshots::VisualSnapshot;
use nve_tables::tables::{
    BackgroundFillRes, BoxShadow, BoxShadows, FlexDirection, Image, ImageFit, ImageSrc, Layout,
    TextContent, Typography, Visibility, Visual,
};
use nve_tables::transform::Transform2D;

use super::oracle_edge_tests::build_redefine_resource_frame;
use super::oracle_tests::{assert_scene_matches_fresh_display_list, physical_dims};
use super::SceneDirtyKind;
use crate::frame_hook::{SkiaFrameHook, TargetsOf};
use crate::image_cache::{encode_rgb_png, ImageCache};
use crate::{SkiaBackend, SkiaSystemPaintCtx, SkiaTableSystem};

#[derive(nve_store::NveTable)]
#[nve_table(
    name = "nve.test.oracle_edge.plugin",
    storage = "sparse",
    flags = "publish"
)]
#[allow(dead_code)]
struct EdgePluginTable {
    #[effect(paint)]
    color: u32,
}

struct EdgePluginSystem;

impl SkiaTableSystem for EdgePluginSystem {
    fn slot(&self) -> TableSlot {
        EdgePluginTable::slot()
    }

    fn paint_node(&self, ctx: &mut SkiaSystemPaintCtx<'_>, store: &NodeStore, node: NodeId) {
        if let Some(row) = store.try_row::<EdgePluginTable>(node) {
            let c = ColorRgba::from_u32(row.color());
            let x0 = (ctx.rect.x.floor().max(0.0) as u32).min(ctx.buffer.width());
            let y0 = (ctx.rect.y.floor().max(0.0) as u32).min(ctx.buffer.height());
            let x1 = ((ctx.rect.x + ctx.rect.width).ceil().max(0.0) as u32).min(ctx.buffer.width());
            let y1 =
                ((ctx.rect.y + ctx.rect.height).ceil().max(0.0) as u32).min(ctx.buffer.height());
            for y in y0..y1 {
                for x in x0..x1 {
                    ctx.buffer.blend_pixel(x as i32, y as i32, c, ctx.opacity);
                }
            }
        }
    }
}

/// Minimal `SkiaFrameHook` exercising both paint (bg + text color) and composite (opacity +
/// transform) dirty collection + `set_reduced_motion` without depending on `nve-motion`.
struct TestAnimHook {
    node: NodeId,
    phase: f32,
    active: bool,
    composite_mode: bool,
}

impl SkiaFrameHook for TestAnimHook {
    fn register_tables(&self) {}
    fn after_commit(&mut self, _: &NodeStore, _: &ChangeSet, _: TargetsOf<'_>) {}
    fn sync_nodes(&mut self, _: &NodeStore, _: &[NodeId], _: TargetsOf<'_>) {}
    fn advance(&mut self, dt_s: f32) -> bool {
        if !self.active || dt_s <= 0.0 {
            return false;
        }
        self.phase = (self.phase + dt_s * 5.0).min(1.0);
        if self.phase >= 1.0 {
            self.active = false;
        }
        true
    }
    fn advance_collecting(
        &mut self,
        dt_s: f32,
        composite_dirty: &mut Vec<NodeId>,
        paint_dirty: &mut Vec<NodeId>,
    ) -> bool {
        if !self.advance(dt_s) {
            return false;
        }
        if self.composite_mode {
            composite_dirty.push(self.node);
        } else {
            paint_dirty.push(self.node);
        }
        true
    }
    fn needs_next_frame(&self) -> bool {
        self.active
    }
    fn set_reduced_motion(&mut self, reduced: bool) -> bool {
        if reduced && self.active {
            self.phase = 1.0;
            self.active = false;
            true
        } else {
            false
        }
    }
    fn set_reduced_motion_collecting(
        &mut self,
        reduced: bool,
        composite_dirty: &mut Vec<NodeId>,
        paint_dirty: &mut Vec<NodeId>,
    ) -> bool {
        if !self.set_reduced_motion(reduced) {
            return false;
        }
        composite_dirty.push(self.node);
        paint_dirty.push(self.node);
        true
    }
    fn apply_visual(&self, node: NodeId, style: &mut VisualSnapshot) {
        if node == self.node && self.phase > 0.0 {
            if self.composite_mode {
                style.opacity = 1.0 - 0.5 * self.phase;
            } else {
                let p = self.phase;
                style.background = ColorRgba::rgb(
                    (20.0 + 200.0 * p) as u8,
                    (40.0 + 160.0 * p) as u8,
                    (80.0 - 40.0 * p) as u8,
                );
                style.text_color = ColorRgba::rgb(
                    (255.0 - 200.0 * p) as u8,
                    240,
                    (255.0 - 75.0 * p) as u8,
                );
            }
        }
    }
    fn overrides_text_color(&self, node: NodeId) -> bool {
        node == self.node && !self.composite_mode && self.phase > 0.0
    }
    fn resolve_transform(&self, node: NodeId, target: Option<Transform2D>) -> Option<Transform2D> {
        if node == self.node && self.composite_mode && self.phase > 0.0 {
            Some(Transform2D::translate(20.0 * self.phase, 0.0))
        } else {
            target
        }
    }
    fn reset(&mut self) {
        self.active = false;
        self.phase = 0.0;
    }
}

#[test]
fn image_cache_invalidation_and_table_plugin_parity() {
    let _img_guard = crate::image_cache::lock_shared_image_cache();
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let mut backend = SkiaBackend::new();
    backend.register_table_system(EdgePluginSystem);
    backend.set_viewport(
        SurfaceId::ROOT,
        Viewport {
            width: 120.0,
            height: 80.0,
            scale_factor: 1.0,
        },
    );

    let png_a = encode_rgb_png(4, 4, &[240, 40, 40].repeat(16)).expect("png a");
    let png_b = encode_rgb_png(4, 4, &[40, 220, 80].repeat(16)).expect("png b");
    let uri_a = format!("data:image/png;base64,{}", encode_base64(&png_a));
    let uri_b = format!("data:image/png;base64,{}", encode_base64(&png_b));

    let (img_node, plug_node) = {
        let mut tx = store.tx();
        tx.edit::<Layout>(NodeId::ROOT)
            .width(Length::points(120.0))
            .height(Length::points(80.0))
            .flex_direction(FlexDirection::Row);
        let img = tx.create();
        tx.edit::<Layout>(img)
            .width(Length::points(50.0))
            .height(Length::points(50.0));
        tx.edit::<Image>(img)
            .src(Some(ImageSrc::Url(uri_a.clone())))
            .fit(ImageFit::Fill);

        let plug = tx.create();
        tx.edit::<Layout>(plug)
            .width(Length::points(50.0))
            .height(Length::points(50.0));
        tx.edit::<EdgePluginTable>(plug)
            .color(ColorRgba::rgb(120, 80, 240).to_u32());

        tx.set_children(NodeId::ROOT, &[img, plug]);
        tx.commit_into(&mut changes);
        (img, plug)
    };
    backend.commit(&store, &changes).expect("init");
    assert_scene_matches_fresh_display_list(&mut backend, &store, 120, 80, "img + plugin init");

    // Decode sibling image into shared cache & clear cache -> live scene must stay identical to oracle.
    let _ = ImageCache::shared().get_or_decode(&uri_b);
    assert_scene_matches_fresh_display_list(&mut backend, &store, 120, 80, "after sibling decode");
    ImageCache::shared().clear();
    assert_scene_matches_fresh_display_list(&mut backend, &store, 120, 80, "after ImageCache clear");

    // Mutate plugin table & swap image src.
    {
        let mut tx = store.tx();
        tx.edit::<EdgePluginTable>(plug_node)
            .color(ColorRgba::rgb(250, 180, 20).to_u32());
        tx.edit::<Image>(img_node)
            .src(Some(ImageSrc::Url(uri_b)));
        tx.commit_into(&mut changes);
    }
    backend.commit(&store, &changes).expect("update img+plugin");
    assert_scene_matches_fresh_display_list(&mut backend, &store, 120, 80, "updated img+plugin");
}

#[test]
fn reparenting_typography_visibility_and_dynamic_hidpi_scale_changes() {
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let mut backend = SkiaBackend::new();
    backend.set_viewport(
        SurfaceId::ROOT,
        Viewport {
            width: 180.0,
            height: 100.0,
            scale_factor: 1.0,
        },
    );

    let (p1, p2, child) = {
        let mut tx = store.tx();
        tx.edit::<Layout>(NodeId::ROOT)
            .width(Length::points(180.0))
            .height(Length::points(100.0))
            .flex_direction(FlexDirection::Row);
        let p1 = tx.create();
        tx.edit::<Layout>(p1)
            .width(Length::points(90.0))
            .height(Length::points(80.0));
        tx.edit::<Visual>(p1).visibility(Visibility::Hidden);
        tx.edit::<Typography>(p1)
            .color(Rgba::rgb(240, 50, 50))
            .font_size(14.0);

        let p2 = tx.create();
        tx.edit::<Layout>(p2)
            .width(Length::points(90.0))
            .height(Length::points(80.0));
        tx.edit::<Visual>(p2).visibility(Visibility::Visible);
        tx.edit::<Typography>(p2)
            .color(Rgba::rgb(50, 230, 120))
            .font_size(11.0);

        let child = tx.create();
        tx.edit::<Layout>(child)
            .width(Length::points(80.0))
            .height(Length::points(30.0));
        tx.edit::<TextContent>(child).text("Moved");
        tx.set_children(p1, &[child]);
        tx.set_children(NodeId::ROOT, &[p1, p2]);
        tx.commit_into(&mut changes);
        (p1, p2, child)
    };
    backend.commit(&store, &changes).expect("init");
    assert_scene_matches_fresh_display_list(&mut backend, &store, 180, 100, "before reparent");

    // Reparent `child` from hidden/red p1 into visible/green p2.
    {
        let mut tx = store.tx();
        tx.set_children(p1, &[]);
        tx.set_children(p2, &[child]);
        tx.commit_into(&mut changes);
    }
    backend.commit(&store, &changes).expect("reparent");
    assert_scene_matches_fresh_display_list(&mut backend, &store, 180, 100, "after reparent");

    // Dynamic HiDPI scale changes: 1.0 -> 2.0 -> 1.25.
    for scale in [2.0f32, 1.25f32] {
        backend.set_viewport(
            SurfaceId::ROOT,
            Viewport {
                width: 180.0,
                height: 100.0,
                scale_factor: scale,
            },
        );
        backend
            .commit(&store, &ChangeSet::default())
            .expect("scale commit");
        assert_scene_matches_fresh_display_list(
            &mut backend,
            &store,
            180,
            100,
            &format!("scale {scale}"),
        );
    }
}

#[test]
fn in_place_resource_redef_gradient_shadow_frame_hook_and_recycled_node_id_aba() {
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let mut backend = SkiaBackend::new();
    backend.set_viewport(
        SurfaceId::ROOT,
        Viewport {
            width: 180.0,
            height: 120.0,
            scale_factor: 1.0,
        },
    );

    let (grad_node, anim_node, comp_node, temp_node) = {
        let mut tx = store.tx();
        tx.edit::<Layout>(NodeId::ROOT)
            .width(Length::points(180.0))
            .height(Length::points(120.0))
            .flex_direction(FlexDirection::Column);

        let grad_node = tx.create();
        tx.edit::<Layout>(grad_node)
            .width(Length::points(120.0))
            .height(Length::points(30.0));
        tx.edit::<Visual>(grad_node)
            .background_fill_ephemeral(BackgroundFillRes(BackgroundFill::linear(
                0.0,
                vec![
                    GradientStop::new(0.0, ColorRgba::rgb(255, 0, 0)),
                    GradientStop::new(1.0, ColorRgba::rgb(0, 0, 255)),
                ],
            )))
            .box_shadows_ephemeral(BoxShadows::new(vec![BoxShadow::new(
                2.0,
                2.0,
                3.0,
                Rgba::new(0, 0, 0, 128),
            )]));

        let anim_node = tx.create();
        tx.edit::<Layout>(anim_node)
            .width(Length::points(120.0))
            .height(Length::points(30.0));
        tx.edit::<Visual>(anim_node).background(Rgba::rgb(20, 40, 80));
        tx.edit::<Typography>(anim_node).color(Rgba::rgb(255, 255, 255));
        tx.edit::<TextContent>(anim_node).text("Color Tick");

        let comp_node = tx.create();
        tx.edit::<Layout>(comp_node)
            .width(Length::points(120.0))
            .height(Length::points(24.0));
        tx.edit::<Visual>(comp_node).background(Rgba::rgb(59, 130, 246));

        let temp_node = tx.create();
        tx.edit::<Layout>(temp_node)
            .width(Length::points(120.0))
            .height(Length::points(24.0));
        tx.edit::<Visual>(temp_node).background(Rgba::rgb(240, 20, 20));
        tx.edit::<TextContent>(temp_node).text("Gen 1");

        tx.set_children(NodeId::ROOT, &[grad_node, anim_node, comp_node, temp_node]);
        tx.commit_into(&mut changes);
        (grad_node, anim_node, comp_node, temp_node)
    };
    backend.register_frame_hook(TestAnimHook {
        node: anim_node,
        phase: 0.0,
        active: true,
        composite_mode: false,
    });
    backend.register_frame_hook(TestAnimHook {
        node: comp_node,
        phase: 0.0,
        active: true,
        composite_mode: true,
    });
    backend.commit(&store, &changes).expect("init");
    assert_scene_matches_fresh_display_list(&mut backend, &store, 180, 120, "res+hook init");

    // 1. In-place gradient & box-shadow resource redefinition via Op::DefineResources.
    let grad_rid = store.row::<Visual>(grad_node).background_fill();
    let shadow_rid = store.row::<Visual>(grad_node).box_shadows();
    let grad_frame = build_redefine_resource_frame(
        grad_rid,
        &BackgroundFillRes(BackgroundFill::linear(
            90.0,
            vec![
                GradientStop::new(0.0, ColorRgba::rgb(16, 185, 129)),
                GradientStop::new(1.0, ColorRgba::rgb(245, 158, 11)),
            ],
        )),
    );
    let shadow_frame = build_redefine_resource_frame(
        shadow_rid,
        &BoxShadows::new(vec![
            BoxShadow::new(4.0, 5.0, 6.0, Rgba::new(255, 0, 128, 204)),
            BoxShadow::new(1.0, 2.0, 3.0, Rgba::new(0, 0, 0, 180)).inset(),
        ]),
    );
    let mut dec = WireDecoder::default();
    {
        let mut tx = store.tx();
        dec.apply(&mut tx, &grad_frame).expect("redef grad");
        dec.apply(&mut tx, &shadow_frame).expect("redef shadow");
        tx.commit_into(&mut changes);
    }
    backend.commit(&store, &changes).expect("in-place res redef");
    assert_scene_matches_fresh_display_list(&mut backend, &store, 180, 120, "in-place grad+shadow");

    // 2. SkiaFrameHook composite + paint ticks & reduced-motion snap.
    assert!(backend.advance_animations(&store, 0.1));
    assert_eq!(
        backend.scene().stats().last_dirty_kind,
        Some(SceneDirtyKind::PaintOnly)
    );
    assert_scene_matches_fresh_display_list(&mut backend, &store, 180, 120, "frame hook tick");
    backend.set_reduced_motion(&store, true);
    assert!(!backend.needs_next_frame());
    assert_scene_matches_fresh_display_list(&mut backend, &store, 180, 120, "reduced motion snap");

    // 3. Recycled NodeId index with bumped generation (ABA safety across frame recycling).
    let old_idx = temp_node.index();
    {
        let mut tx = store.tx();
        tx.remove(temp_node);
        tx.set_children(NodeId::ROOT, &[grad_node, anim_node, comp_node]);
        tx.commit_into(&mut changes);
    }
    backend.commit(&store, &changes).expect("aba remove");
    assert!(backend.scene().live_chunk_idx(temp_node).is_none());

    let recycled_node = {
        let mut tx = store.tx();
        let replacement = tx.create();
        tx.edit::<Layout>(replacement)
            .width(Length::points(120.0))
            .height(Length::points(24.0));
        tx.edit::<Visual>(replacement).background(Rgba::rgb(34, 197, 94));
        tx.edit::<TextContent>(replacement).text("Gen 2");
        tx.set_children(
            NodeId::ROOT,
            &[grad_node, anim_node, comp_node, replacement],
        );
        tx.commit_into(&mut changes);
        replacement
    };
    backend.commit(&store, &changes).expect("aba replace");
    assert_eq!(
        recycled_node.index(),
        old_idx,
        "test setup expects NodeStore to recycle freed slot index"
    );
    assert_ne!(recycled_node, temp_node);
    assert!(
        backend.scene().live_chunk_idx(temp_node).is_none(),
        "stale generation NodeId MUST NOT alias live recycled chunk"
    );
    assert!(backend.scene().live_chunk_idx(recycled_node).is_some());
    assert_scene_matches_fresh_display_list(&mut backend, &store, 180, 120, "recycled NodeId ABA");

    // Keep physical_dims helper explicitly exercised.
    assert_eq!(physical_dims(180, 120, 1.5), (270, 180));
}

fn encode_base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((n >> 18) & 63) as usize] as char);
        out.push(TABLE[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            TABLE[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}
