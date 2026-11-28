//! Regression guards for Skia plugin cache invalidation (scale/reset/reload/late registration),
//! resource-driven plugin updates, and non-finite `DisplayList` / caret / `-0.0` key safety.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use nve_abi::{ColorRgba, EventBatchBuf, Key64, SurfaceId};
use nve_engine::{LayoutRect, StoreBackend, Viewport};
use nve_store::{
    ChangeSet, FlatResource, Length, NodeId, NodeStore, NveTable, Res, Rgba, TableSlot,
    ValidatedWords,
};
use nve_tables::paint::{BackgroundFill, GradientStop, LinearGradient, RadialGradient};
use nve_tables::tables::{ImageFit, Layout};

use super::cache_parity_tests::{CountingPluginSystem, ParityPlugin};
use super::super::*;
use crate::table_system::{SkiaSystemPaintCtx, SkiaTableRegistry, SkiaTableSystem};
use crate::text::{CosmicTextEngine, TextAlign};
use crate::{DisplayList, PaintOp, SkiaBackend};

#[derive(Debug, Clone, PartialEq, Eq)]
struct TestResPayload(u32);

impl FlatResource for TestResPayload {
    const KIND: Key64 = Key64::of("nve.test.cache_invalidation.res");
    type View<'a> = u32;

    fn write_words(&self, out: &mut Vec<u32>) -> bool {
        out.push(self.0);
        true
    }

    fn validate(words: &[u32]) -> bool {
        words.len() == 1
    }

    fn view(words: ValidatedWords<'_, Self>) -> u32 {
        words.as_slice()[0]
    }
}

#[derive(NveTable)]
#[nve_table(name = "nve.test.static_vector_4b", storage = "sparse")]
#[allow(dead_code)]
struct StaticVectorPlugin {
    #[effect(paint)]
    pub payload: Res<TestResPayload>,
}

#[derive(NveTable)]
#[nve_table(name = "nve.test.anim_canvas_4b", storage = "sparse")]
#[allow(dead_code)]
struct AnimCanvasPlugin {
    #[effect(paint)]
    pub payload: Res<TestResPayload>,
}

struct ResCountingSystem {
    slot: TableSlot,
    calls: Arc<AtomicU32>,
    color: ColorRgba,
}

impl SkiaTableSystem for ResCountingSystem {
    fn slot(&self) -> TableSlot {
        self.slot
    }

    fn paint_node(&self, ctx: &mut SkiaSystemPaintCtx<'_>, _store: &NodeStore, _node: NodeId) {
        self.calls.fetch_add(1, Ordering::Relaxed);
        let w = (ctx.rect.width.round() as i32).clamp(1, 8);
        let h = (ctx.rect.height.round() as i32).clamp(1, 8);
        for y in 0..h {
            for x in 0..w {
                ctx.buffer.blend_pixel(x, y, self.color, 1.0);
            }
        }
    }
}

#[test]
fn plugin_cache_invalidates_on_scale_reset_dylib_reload_and_late_system_registration() {
    let make_store = |color: ColorRgba, width: f32, height: f32| {
        let mut store = NodeStore::new();
        let mut changes = ChangeSet::default();
        let node = {
            let mut tx = store.tx();
            let n = tx.create();
            tx.set_children(NodeId::ROOT, &[n]);
            tx.edit::<Layout>(n)
                .width(Length::points(width))
                .height(Length::points(height));
            tx.edit::<ParityPlugin>(n).color_rgba(color.to_u32());
            tx.commit_into(&mut changes);
            n
        };
        (store, changes, node)
    };

    let calls = Arc::new(AtomicU32::new(0));
    let mut surface = CpuSkiaSurface::new(220, 220).unwrap();
    let mut buf = PixelBuffer::new(220, 220);

    // (a) 100x100 @ 2x vs 200x200 @ 1x: identical (200x200) pixel size, DIFFERENT eff_scale.
    let (mut store, mut changes, node) = make_store(ColorRgba::rgb(255, 0, 0), 100.0, 100.0);
    let mut backend = SkiaBackend::new();
    backend.register_table_system(CountingPluginSystem {
        calls: Arc::clone(&calls),
    });
    backend.set_viewport(
        SurfaceId::ROOT,
        Viewport {
            width: 220.0,
            height: 220.0,
            scale_factor: 2.0,
        },
    );
    backend.commit(&store, &changes).unwrap();
    {
        let (scene, reg, ev) = backend.scene_paint_parts_mut();
        surface
            .render_scene_cached_with_plugins(
                scene,
                2.0,
                ColorRgba::BLACK,
                Some((&store, reg, ev)),
            )
            .unwrap();
    }
    assert_eq!(calls.load(Ordering::Relaxed), 1);

    {
        let mut tx = store.tx();
        tx.edit::<Layout>(node)
            .width(Length::points(200.0))
            .height(Length::points(200.0));
        tx.commit_into(&mut changes);
    }
    backend.set_viewport(
        SurfaceId::ROOT,
        Viewport {
            width: 220.0,
            height: 220.0,
            scale_factor: 1.0,
        },
    );
    backend.commit(&store, &changes).unwrap();
    surface.reset_cache_stats();
    {
        let (scene, reg, ev) = backend.scene_paint_parts_mut();
        surface
            .render_scene_cached_with_plugins(
                scene,
                1.0,
                ColorRgba::BLACK,
                Some((&store, reg, ev)),
            )
            .unwrap();
    }
    assert_eq!(surface.cache_stats().plugin_rasterizations, 1);

    // (b) Reset + recreate same NodeId with different color: monotonic epoch prevents collision.
    backend.reset();
    let (store_b, changes_b, _) = make_store(ColorRgba::rgb(0, 255, 0), 200.0, 200.0);
    backend.set_viewport(
        SurfaceId::ROOT,
        Viewport {
            width: 220.0,
            height: 220.0,
            scale_factor: 1.0,
        },
    );
    backend.commit(&store_b, &changes_b).unwrap();
    surface.reset_cache_stats();
    {
        let (scene, reg, ev) = backend.scene_paint_parts_mut();
        surface
            .render_scene_cached_with_plugins(
                scene,
                1.0,
                ColorRgba::BLACK,
                Some((&store_b, reg, ev)),
            )
            .unwrap();
    }
    assert_eq!(surface.cache_stats().plugin_rasterizations, 1);
    surface.copy_to_pixel_buffer(&mut buf).unwrap();
    assert_eq!(buf.get_rgb(50, 50), Some((0, 255, 0)));

    // (c) Fresh `SkiaBackend::new()` swap (Tier-2 DylibReload) against SAME reused `CpuSkiaSurface`.
    let (store_c, changes_c, _) = make_store(ColorRgba::rgb(0, 0, 255), 200.0, 200.0);
    let mut reloaded = SkiaBackend::new();
    reloaded.register_table_system(CountingPluginSystem {
        calls: Arc::clone(&calls),
    });
    reloaded.set_viewport(
        SurfaceId::ROOT,
        Viewport {
            width: 220.0,
            height: 220.0,
            scale_factor: 1.0,
        },
    );
    reloaded.commit(&store_c, &changes_c).unwrap();
    surface.reset_cache_stats();
    {
        let (scene, reg, ev) = reloaded.scene_paint_parts_mut();
        surface
            .render_scene_cached_with_plugins(
                scene,
                1.0,
                ColorRgba::BLACK,
                Some((&store_c, reg, ev)),
            )
            .unwrap();
    }
    assert_eq!(surface.cache_stats().plugin_rasterizations, 1);
    surface.copy_to_pixel_buffer(&mut buf).unwrap();
    assert_eq!(buf.get_rgb(50, 50), Some((0, 0, 255)));

    // (d) Non-vacuous negative `image: None` cache invalidation: paint a DisplayList with
    //     `PaintOp::Plugin` against an EMPTY registry first (seeding negative cache entry), then
    //     register the plugin on that exact registry without rebuilding `DisplayList`.
    let (store_d, _, node_d) = make_store(ColorRgba::rgb(255, 255, 0), 200.0, 200.0);
    let dl_d = DisplayList {
        ops: vec![PaintOp::Plugin {
            node_id: node_d,
            rect: LayoutRect {
                x: 0.0,
                y: 0.0,
                width: 200.0,
                height: 200.0,
            },
            inherited_text_color: Rgba::WHITE,
            plugin_epoch: 1,
        }],
    };
    let mut empty_reg = SkiaTableRegistry::new();
    let mut ev_d = EventBatchBuf::new();
    let mut neg_surface = CpuSkiaSurface::new(220, 220).unwrap();
    neg_surface
        .render_cached_with_plugins(
            &dl_d,
            1.0,
            ColorRgba::BLACK,
            Some((&store_d, &empty_reg, &mut ev_d)),
        )
        .unwrap();
    assert_eq!(neg_surface.cache_stats().plugin_entries, 1);
    assert_eq!(neg_surface.cache_stats().plugin_rasterizations, 1);
    neg_surface.copy_to_pixel_buffer(&mut buf).unwrap();
    assert_eq!(buf.get_rgb(50, 50), Some((0, 0, 0)));

    empty_reg.register(CountingPluginSystem {
        calls: Arc::clone(&calls),
    });
    neg_surface.reset_cache_stats();
    neg_surface
        .render_cached_with_plugins(
            &dl_d,
            1.0,
            ColorRgba::BLACK,
            Some((&store_d, &empty_reg, &mut ev_d)),
        )
        .unwrap();
    assert_eq!(neg_surface.cache_stats().plugin_rasterizations, 1);
    neg_surface.copy_to_pixel_buffer(&mut buf).unwrap();
    assert_eq!(buf.get_rgb(50, 50), Some((255, 255, 0)));
}

#[test]
fn animating_resource_plugin_overwrites_in_place_and_never_invalidates_static_sibling() {
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let anim_node = {
        let mut tx = store.tx();
        let s_node = tx.create();
        let a_node = tx.create();
        tx.set_children(NodeId::ROOT, &[s_node, a_node]);
        tx.edit::<Layout>(s_node)
            .width(Length::points(40.0))
            .height(Length::points(40.0));
        tx.edit::<StaticVectorPlugin>(s_node)
            .payload(Some(TestResPayload(1)));
        tx.edit::<Layout>(a_node)
            .width(Length::points(40.0))
            .height(Length::points(40.0));
        tx.edit::<AnimCanvasPlugin>(a_node)
            .payload_ephemeral(TestResPayload(100));
        tx.commit_into(&mut changes);
        a_node
    };

    let static_calls = Arc::new(AtomicU32::new(0));
    let anim_calls = Arc::new(AtomicU32::new(0));
    let mut backend = SkiaBackend::new();
    backend.register_table_system(ResCountingSystem {
        slot: StaticVectorPlugin::slot(),
        calls: Arc::clone(&static_calls),
        color: ColorRgba::rgb(16, 185, 129),
    });
    backend.register_table_system(ResCountingSystem {
        slot: AnimCanvasPlugin::slot(),
        calls: Arc::clone(&anim_calls),
        color: ColorRgba::rgb(239, 68, 68),
    });
    backend.set_viewport(
        SurfaceId::ROOT,
        Viewport {
            width: 120.0,
            height: 120.0,
            scale_factor: 1.0,
        },
    );
    backend.commit(&store, &changes).unwrap();

    let mut surface = CpuSkiaSurface::new(120, 120).unwrap();
    {
        let (scene, reg, ev) = backend.scene_paint_parts_mut();
        surface
            .render_scene_cached_with_plugins(
                scene,
                1.0,
                ColorRgba::BLACK,
                Some((&store, reg, ev)),
            )
            .unwrap();
    }

    for frame in 1..100u32 {
        {
            let mut tx = store.tx();
            tx.edit::<AnimCanvasPlugin>(anim_node)
                .payload_ephemeral(TestResPayload(100 + frame));
            tx.commit_into(&mut changes);
        }
        backend.commit(&store, &changes).unwrap();
        let (scene, reg, ev) = backend.scene_paint_parts_mut();
        surface
            .render_scene_cached_with_plugins(
                scene,
                1.0,
                ColorRgba::BLACK,
                Some((&store, reg, ev)),
            )
            .unwrap();
    }

    assert_eq!(anim_calls.load(Ordering::Relaxed), 100);
    assert_eq!(static_calls.load(Ordering::Relaxed), 1);
    assert!(surface.cache_stats().plugin_entries <= 2);
}

#[test]
fn non_finite_display_list_ops_and_negative_zero_text_keys_render_safely() {
    let nan_rect = LayoutRect {
        x: f32::NAN,
        y: f32::INFINITY,
        width: f32::NEG_INFINITY,
        height: -10.0,
    };
    let all_nan_rect = LayoutRect {
        x: f32::NAN,
        y: f32::NAN,
        width: f32::NAN,
        height: f32::NAN,
    };
    let dl = DisplayList {
        ops: vec![
            PaintOp::FillRoundedRect {
                rect: all_nan_rect,
                radius: f32::NAN,
                color: ColorRgba::WHITE,
            },
            PaintOp::StrokeRoundedRect {
                rect: all_nan_rect,
                radius: f32::NAN,
                width: 2.0,
                color: ColorRgba::WHITE,
            },
            PaintOp::DrawText {
                rect: all_nan_rect,
                text: "All NaN rect".to_owned(),
                font_size: 14.0,
                font_weight: 400,
                text_align: TextAlign::Left,
                color: ColorRgba::WHITE,
            },
            PaintOp::DrawShadow {
                rect: nan_rect,
                radius: f32::NAN,
                offset_x: f32::INFINITY,
                offset_y: f32::NEG_INFINITY,
                blur_radius: f32::NAN,
                spread_radius: f32::INFINITY,
                color: ColorRgba::WHITE,
                border_width: f32::NAN,
                inset: false,
            },
            PaintOp::DrawShadow {
                rect: nan_rect,
                radius: f32::NAN,
                offset_x: f32::INFINITY,
                offset_y: f32::NEG_INFINITY,
                blur_radius: f32::NAN,
                spread_radius: f32::INFINITY,
                color: ColorRgba::WHITE,
                border_width: f32::NAN,
                inset: true,
            },
            PaintOp::FillRoundedRect {
                rect: nan_rect,
                radius: f32::INFINITY,
                color: ColorRgba::WHITE,
            },
            PaintOp::StrokeRoundedRect {
                rect: nan_rect,
                radius: f32::NAN,
                width: f32::INFINITY,
                color: ColorRgba::WHITE,
            },
            PaintOp::FillRoundedRectGradient {
                rect: nan_rect,
                radius: f32::NAN,
                gradient: BackgroundFill::Linear(LinearGradient::new(
                    f32::NAN,
                    vec![
                        GradientStop::new(f32::NAN, ColorRgba::WHITE),
                        GradientStop::new(f32::INFINITY, ColorRgba::BLACK),
                    ],
                )),
            },
            PaintOp::FillRoundedRectGradient {
                rect: LayoutRect {
                    x: 4.0,
                    y: 4.0,
                    width: 40.0,
                    height: 40.0,
                },
                radius: 4.0,
                gradient: BackgroundFill::Radial(RadialGradient::new(
                    f32::NAN,
                    f32::INFINITY,
                    -5.0,
                    vec![GradientStop::new(0.0, ColorRgba::rgb(255, 0, 0))],
                )),
            },
            PaintOp::PushClip {
                rect: nan_rect,
                radius: f32::NAN,
            },
            PaintOp::PopClip,
            // Draw solid rects INSIDE and AFTER `SaveOpacity(NAN)` and `PushTransform([NAN; 6])`
            // so we verify both non-finite sanitization AND intact CTM / save-restore balance.
            PaintOp::SaveOpacity(f32::NAN),
            PaintOp::FillRoundedRect {
                rect: LayoutRect {
                    x: 4.0,
                    y: 4.0,
                    width: 12.0,
                    height: 12.0,
                },
                radius: 0.0,
                color: ColorRgba::rgb(255, 0, 0),
            },
            PaintOp::Restore,
            PaintOp::FillRoundedRect {
                rect: LayoutRect {
                    x: 20.0,
                    y: 4.0,
                    width: 12.0,
                    height: 12.0,
                },
                radius: 0.0,
                color: ColorRgba::rgb(0, 255, 0),
            },
            PaintOp::PushTransform {
                matrix: [f32::NAN; 6],
            },
            PaintOp::FillRoundedRect {
                rect: LayoutRect {
                    x: 4.0,
                    y: 20.0,
                    width: 12.0,
                    height: 12.0,
                },
                radius: 0.0,
                color: ColorRgba::rgb(0, 0, 255),
            },
            PaintOp::PopTransform,
            PaintOp::FillRoundedRect {
                rect: LayoutRect {
                    x: 20.0,
                    y: 20.0,
                    width: 12.0,
                    height: 12.0,
                },
                radius: 0.0,
                color: ColorRgba::rgb(255, 255, 0),
            },
            PaintOp::DrawImage {
                rect: nan_rect,
                radius: f32::NAN,
                src: "nve-test://missing".to_owned(),
                fit: ImageFit::Cover,
                resolved: None,
            },
            PaintOp::DrawText {
                rect: nan_rect,
                text: "NaN guard".to_owned(),
                font_size: f32::NAN,
                font_weight: 400,
                text_align: TextAlign::Center,
                color: ColorRgba::WHITE,
            },
        ],
    };

    let mut surface = CpuSkiaSurface::new(64, 64).unwrap();
    surface
        .render_cached_with_plugins(&dl, f32::NAN, ColorRgba::BLACK, None)
        .expect("non-finite ops must not panic");
    let mut skia_buf = PixelBuffer::new(64, 64);
    surface.copy_to_pixel_buffer(&mut skia_buf).unwrap();
    assert_eq!(skia_buf.get_rgb(10, 10), Some((255, 0, 0)));
    assert_eq!(skia_buf.get_rgb(26, 10), Some((0, 255, 0)));
    assert_eq!(skia_buf.get_rgb(10, 26), Some((0, 0, 255)));
    assert_eq!(skia_buf.get_rgb(26, 26), Some((255, 255, 0)));

    let mut soft_buf = PixelBuffer::new(64, 64);
    crate::raster::rasterize_display_list(&dl, &mut soft_buf, f32::NAN, ColorRgba::BLACK);
    assert_eq!(soft_buf.get_rgb(10, 10), Some((255, 0, 0)));
    assert_eq!(soft_buf.get_rgb(26, 10), Some((0, 255, 0)));
    assert_eq!(soft_buf.get_rgb(10, 26), Some((0, 0, 255)));
    assert_eq!(soft_buf.get_rgb(26, 26), Some((255, 255, 0)));

    // Also exercise fontless bitmap text path with all-NaN rect & opacity.
    let fontless = CosmicTextEngine::new_without_system_fonts();
    crate::raster::rasterize_display_list_with_text_engine(
        &dl,
        &mut soft_buf,
        1.0,
        ColorRgba::BLACK,
        &fontless,
    );

    // Exercise NaN caret geometry snapping (must return finite zero rect without panicking).
    let caret = crate::text_input::compute_ime_cursor_rect(
        &fontless,
        "Hello",
        2,
        None,
        14.0,
        400,
        false,
        all_nan_rect,
        f32::NAN,
    );
    assert!(
        caret.x.is_finite()
            && caret.y.is_finite()
            && caret.width.is_finite()
            && caret.height.is_finite()
    );
    let snapped = crate::retained::replay::snap_world_rect(
        all_nan_rect,
        f32::NAN,
        f32::NAN,
        f32::NAN,
        f32::NAN,
        f32::NAN,
    );
    assert!(
        snapped.x.is_finite()
            && snapped.y.is_finite()
            && snapped.width.is_finite()
            && snapped.height.is_finite()
    );

    let renderer = SkiaCanvasRenderer::new();
    let _ = renderer.build_cached_text_blob("NegZero", 16.0, 400, TextAlign::Left, Some(0.0), 0.0);
    renderer.reset_cache_stats();
    let _ =
        renderer.build_cached_text_blob("NegZero", 16.0, 400, TextAlign::Left, Some(-0.0), -0.0);
    let stats = renderer.cache_stats();
    assert_eq!(stats.text_blob_misses, 0);
    assert_eq!(stats.text_blob_hits, 1);
}
