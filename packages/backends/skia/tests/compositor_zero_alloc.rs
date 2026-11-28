//! Strict zero-allocation (`CountingAlloc`) guards for Slice 4C retained `CompositorScene`
//! 120 Hz `COMPOSITE` hot paths (scroll with sticky/opacity/transform/frame-hook, opacity+transform
//! commits including on hook-tracked nodes, in-place `z_index` reorder, `SkiaFrameHook`
//! opacity+transform animation ticks, and software/Skia replay).
//!
//! Run with `--test-threads=1` so the global allocator counter observes only this test's thread.

use std::alloc::{GlobalAlloc, Layout as AllocLayout, System};
use std::cell::Cell;
use std::hint::black_box;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use nve_abi::{ColorRgba, SurfaceId};
use nve_engine::{StoreBackend, Viewport};
use nve_store::{ChangeSet, Length, NodeId, NodeStore, NveTable, Rgba, TableSlot};
use nve_tables::paint::{BackgroundFill, GradientStop};
use nve_tables::snapshots::VisualSnapshot;
use nve_tables::tables::{
    BackgroundFillRes, BoxShadow, BoxShadows, FlexDirection, Image, ImageFit, ImageSrc, Layout,
    Overflow, Position, Scroll, TextContent, Transform, Typography, Visual,
};
use nve_tables::transform::Transform2D;

use nve_backend_skia::image_cache::ImageCache;
use nve_backend_skia::raster::PixelBuffer;
use nve_backend_skia::{
    SceneDirtyKind, SkiaBackend, SkiaFrameHook, SkiaSystemPaintCtx, SkiaTableSystem, TargetsOf,
};

struct CountingAlloc;

thread_local! {
    static COUNTING: Cell<bool> = const { Cell::new(false) };
}
static ALLOC_COUNT: AtomicUsize = AtomicUsize::new(0);
static SERIAL_TEST: Mutex<()> = Mutex::new(());

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

// SAFETY: Delegates directly to the system allocator; `try_with` isolates per-thread counting
// without ever panicking during TLS teardown inside `GlobalAlloc`.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: AllocLayout) -> *mut u8 {
        if COUNTING.try_with(Cell::get).unwrap_or(false) {
            ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        }
        // SAFETY: Delegates directly to the system allocator with caller-provided layout.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: AllocLayout) {
        // SAFETY: Delegates directly to the system allocator for a pointer allocated by `System`.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: AllocLayout, new_size: usize) -> *mut u8 {
        if COUNTING.try_with(Cell::get).unwrap_or(false) {
            ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        }
        // SAFETY: Delegates directly to the system allocator with caller-provided invariants.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

fn measure_allocs<F: FnOnce()>(f: F) -> usize {
    ALLOC_COUNT.store(0, Ordering::SeqCst);
    let _ = COUNTING.try_with(|c| c.set(true));
    f();
    let _ = COUNTING.try_with(|c| c.set(false));
    ALLOC_COUNT.load(Ordering::SeqCst)
}

/// Lightweight local `SkiaFrameHook` driving composite-only (`opacity` + `transform`) animations
/// without depending on `nve-motion`.
struct TestCompositeHook {
    target: NodeId,
    active: bool,
    opacity: f32,
    tx: f32,
}

impl TestCompositeHook {
    fn new(target: NodeId) -> Self {
        Self { target, active: false, opacity: 1.0, tx: 0.0 }
    }
}

impl SkiaFrameHook for TestCompositeHook {
    fn register_tables(&self) {}

    fn after_commit(&mut self, _store: &NodeStore, changes: &ChangeSet, targets: TargetsOf<'_>) {
        if changes.touched.contains(&self.target) {
            let t = targets(self.target);
            if (t.visual.opacity - self.opacity).abs() > 1e-4
                || (t.transform.translate_x - self.tx).abs() > 1e-4
            {
                self.active = true;
            }
        }
    }

    fn sync_nodes(&mut self, _store: &NodeStore, _dirty: &[NodeId], _targets: TargetsOf<'_>) {}

    fn advance(&mut self, dt_s: f32) -> bool {
        if !self.active || !(dt_s.is_finite() && dt_s > 0.0) {
            return false;
        }
        self.opacity = (self.opacity - dt_s * 1.2).max(0.2);
        self.tx = (self.tx + dt_s * 60.0).min(40.0);
        true
    }

    fn advance_collecting(
        &mut self,
        dt_s: f32,
        composite_dirty: &mut Vec<NodeId>,
        _paint_dirty: &mut Vec<NodeId>,
    ) -> bool {
        if self.advance(dt_s) {
            composite_dirty.push(self.target);
            true
        } else {
            false
        }
    }

    fn needs_next_frame(&self) -> bool { self.active }
    fn set_reduced_motion(&mut self, _reduced: bool) -> bool { false }

    fn apply_visual(&self, node: NodeId, style: &mut VisualSnapshot) {
        if self.active && node == self.target {
            style.opacity = self.opacity;
        }
    }

    fn resolve_transform(&self, node: NodeId, target: Option<Transform2D>) -> Option<Transform2D> {
        if self.active && node == self.target {
            Some(Transform2D::translate(self.tx, 0.0))
        } else {
            target
        }
    }

    fn reset(&mut self) {
        self.active = false;
        self.opacity = 1.0;
        self.tx = 0.0;
    }
}

/// Rich 1,000-node scroll scene with `Position::Sticky`, translucent, transformed, and hook nodes.
fn build_1000_node_rich_scroll_scene() -> (
    NodeStore,
    ChangeSet,
    SkiaBackend,
    NodeId,
    NodeId,
    NodeId,
    Vec<NodeId>,
) {
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let mut backend = SkiaBackend::new();
    backend.set_viewport(
        SurfaceId::ROOT,
        Viewport { width: 320.0, height: 240.0, scale_factor: 1.0 },
    );

    let (scroll, sticky, anim_node, cards) = {
        let mut tx = store.tx();
        tx.edit::<Layout>(NodeId::ROOT).width(Length::points(320.0)).height(Length::points(240.0));
        let scroll = tx.create();
        tx.edit::<Layout>(scroll)
            .width(Length::points(320.0))
            .height(Length::points(240.0))
            .flex_direction(FlexDirection::Column);
        tx.edit::<Scroll>(scroll);
        tx.edit::<Visual>(scroll).overflow_y(Overflow::Scroll);

        let sticky = tx.create();
        tx.edit::<Layout>(sticky)
            .position(Position::Sticky)
            .inset_top(Length::points(0.0))
            .width(Length::points(300.0))
            .height(Length::points(24.0))
            .flex_shrink(0.0);
        tx.edit::<Visual>(sticky).background(Rgba::rgb(30, 58, 138)).z_index(10);

        let mut children = Vec::with_capacity(1001);
        children.push(sticky);
        let mut cards = Vec::with_capacity(1000);
        for i in 0..1000usize {
            let c = tx.create();
            tx.edit::<Layout>(c)
                .width(Length::points(300.0))
                .height(Length::points(20.0))
                .flex_shrink(0.0);
            let v = tx.edit::<Visual>(c)
                .background(Rgba::rgb((i & 255) as u8, 80, 140))
                .border_radius(4.0);
            if i % 8 == 1 {
                v.opacity(0.75);
            }
            if i % 8 == 2 {
                tx.edit::<Transform>(c).translate_x(3.0);
            }
            children.push(c);
            cards.push(c);
        }

        let anim_node = cards[3];
        tx.edit::<Visual>(anim_node).opacity(1.0);
        tx.edit::<Transform>(anim_node).translate_x(0.0);

        tx.set_children(scroll, &children);
        tx.set_children(NodeId::ROOT, &[scroll]);
        tx.commit_into(&mut changes);
        (scroll, sticky, anim_node, cards)
    };
    backend.register_frame_hook(TestCompositeHook::new(anim_node));
    backend.commit(&store, &changes).expect("initial 1k commit");
    (store, changes, backend, scroll, sticky, anim_node, cards)
}

#[test]
fn counting_alloc_probe_detects_real_heap_allocation() {
    let _serial = SERIAL_TEST.lock().unwrap_or_else(|e| e.into_inner());
    let allocs = measure_allocs(|| {
        let v: Vec<u8> = black_box(vec![42u8; 128]);
        black_box(v);
    });
    assert!(allocs >= 1, "CountingAlloc positive probe must observe heap allocs, got {allocs}");
}

#[test]
fn scroll_120hz_fast_path_and_software_replay_zero_allocs() {
    let _serial = SERIAL_TEST.lock().unwrap_or_else(|e| e.into_inner());
    let (store, _changes, mut backend, scroll, sticky, _anim_node, _cards) =
        build_1000_node_rich_scroll_scene();
    let mut buf = PixelBuffer::new(320, 240);

    backend.set_scroll_offset(&store, scroll, 0.0, 16.0);
    backend.rasterize_with_store(&store, &mut buf, ColorRgba::BLACK);
    let pixels_before = buf.pixels().to_vec();
    let emitted_before = backend.scene().stats().chunks_emitted;
    let fast_before = backend.scene().stats().scroll_fast_paths;

    let allocs = measure_allocs(|| {
        for step in 1..=10u32 {
            backend.set_scroll_offset(&store, scroll, 0.0, (step as f32) * 32.0);
            backend.rasterize_with_store(&store, &mut buf, ColorRgba::BLACK);
        }
    });

    assert_eq!(backend.scene().stats().scroll_fast_paths, fast_before + 10);
    assert_eq!(backend.scene().stats().chunks_emitted, emitted_before);
    assert_ne!(buf.pixels(), pixels_before.as_slice());
    let sticky_tf = backend.scene().transform_for_node(sticky).expect("sticky chunk live");
    assert!(sticky_tf.sticky_y > 0.0, "sticky header must update (got {})", sticky_tf.sticky_y);
    assert_eq!(allocs, 0, "10x 120 Hz scroll + software replay must perform 0 allocs, got {allocs}");
}

#[test]
fn composite_only_opacity_transform_and_hook_node_commit_zero_allocs() {
    let _serial = SERIAL_TEST.lock().unwrap_or_else(|e| e.into_inner());
    let (mut store, mut changes, mut backend, _scroll, _sticky, anim_node, cards) =
        build_1000_node_rich_scroll_scene();
    let target = cards[0];

    {
        let mut tx = store.tx();
        tx.edit::<Visual>(target).opacity(0.9);
        tx.edit::<Transform>(target).translate_x(4.0);
        tx.edit::<Visual>(anim_node).opacity(0.8);
        tx.edit::<Transform>(anim_node).translate_x(6.0);
        tx.commit_into(&mut changes);
    }
    backend.commit(&store, &changes).expect("warmup composite");
    let emitted_before = backend.scene().stats().chunks_emitted;
    let comp_before = backend.scene().stats().composite_fast_paths;

    let allocs = measure_allocs(|| {
        for step in 1..=10u32 {
            let mut tx = store.tx();
            tx.edit::<Visual>(target).opacity(0.5 + (step as f32) * 0.04);
            tx.edit::<Transform>(target).translate_x((step as f32) * 2.0);
            tx.edit::<Visual>(anim_node).opacity(0.4 + (step as f32) * 0.05);
            tx.edit::<Transform>(anim_node).translate_x((step as f32) * 3.0);
            tx.commit_into(&mut changes);
            backend.commit(&store, &changes).expect("composite commit succeeds");
        }
    });

    assert_eq!(backend.scene().stats().last_dirty_kind, Some(SceneDirtyKind::CompositeOnly));
    assert_eq!(backend.scene().stats().composite_fast_paths, comp_before + 10);
    assert_eq!(backend.scene().stats().chunks_emitted, emitted_before);
    let eff = backend.scene().effect_for_node(target).expect("target effect live");
    assert!((eff.opacity - 0.9).abs() < 1e-4, "opacity mismatch: {}", eff.opacity);
    let tf = backend.scene().transform_for_node(target).expect("target transform live");
    assert!((tf.local_transform.translate_x - 20.0).abs() < 1e-4);
    assert_eq!(allocs, 0, "10x COMPOSITE-only commits must perform 0 allocs, got {allocs}");
}

#[test]
fn composite_only_z_index_reorder_1000_nodes_zero_allocs() {
    let _serial = SERIAL_TEST.lock().unwrap_or_else(|e| e.into_inner());
    let (mut store, mut changes, mut backend, _scroll, _sticky, _anim_node, cards) =
        build_1000_node_rich_scroll_scene();
    let target = cards[0];

    {
        let mut tx = store.tx();
        tx.edit::<Visual>(target).z_index(2);
        tx.commit_into(&mut changes);
    }
    backend.commit(&store, &changes).expect("warmup z_index");
    let emitted_before = backend.scene().stats().chunks_emitted;
    let children_len_before = backend.scene().ordered_children_len();

    let allocs = measure_allocs(|| {
        for step in 1..=10i32 {
            let mut tx = store.tx();
            tx.edit::<Visual>(target).z_index(if step % 2 == 0 { 5 } else { -1 });
            tx.commit_into(&mut changes);
            backend.commit(&store, &changes).expect("z_index composite commit succeeds");
        }
    });

    assert_eq!(backend.scene().stats().last_dirty_kind, Some(SceneDirtyKind::CompositeOnly));
    assert_eq!(backend.scene().stats().chunks_emitted, emitted_before);
    assert_eq!(backend.scene().ordered_children_len(), children_len_before);
    assert_eq!(backend.scene().chunk_for_node(target).expect("live").z_index, 5);
    assert_eq!(allocs, 0, "10x z_index reorders must perform 0 allocs, got {allocs}");
}

#[test]
fn advance_animations_opacity_and_transform_120hz_zero_allocs() {
    let _serial = SERIAL_TEST.lock().unwrap_or_else(|e| e.into_inner());
    let (mut store, mut changes, mut backend, _scroll, _sticky, anim_node, _cards) =
        build_1000_node_rich_scroll_scene();
    let mut buf = PixelBuffer::new(320, 240);

    {
        let mut tx = store.tx();
        tx.edit::<Visual>(anim_node).opacity(0.2);
        tx.edit::<Transform>(anim_node).translate_x(40.0);
        tx.commit_into(&mut changes);
    }
    backend.commit(&store, &changes).expect("trigger hook");
    assert!(backend.needs_next_frame());

    assert!(backend.advance_animations(&store, 1.0 / 120.0));
    backend.rasterize_with_store(&store, &mut buf, ColorRgba::BLACK);
    let emitted_before = backend.scene().stats().chunks_emitted;
    let op_before = backend.scene().effect_for_node(anim_node).expect("live").opacity;

    let allocs = measure_allocs(|| {
        for _ in 0..10 {
            assert!(backend.advance_animations(&store, 1.0 / 120.0));
            backend.rasterize_with_store(&store, &mut buf, ColorRgba::BLACK);
        }
    });

    let op_after = backend.scene().effect_for_node(anim_node).expect("live").opacity;
    assert!(op_after < op_before, "animated opacity must progress ({op_before} -> {op_after})");
    assert_eq!(backend.scene().stats().chunks_emitted, emitted_before);
    assert_eq!(allocs, 0, "10x animation ticks + software replay must perform 0 allocs, got {allocs}");
}

#[derive(NveTable)]
#[nve_table(name = "nve.test.compositor_zero_alloc.plugin", storage = "sparse")]
#[allow(dead_code)]
struct RichPluginTable {
    #[effect(paint)]
    color: u32,
}

struct RichPluginSystem;

impl SkiaTableSystem for RichPluginSystem {
    fn slot(&self) -> TableSlot {
        RichPluginTable::slot()
    }
    fn paint_node(&self, ctx: &mut SkiaSystemPaintCtx<'_>, store: &NodeStore, node: NodeId) {
        if let Some(row) = store.try_row::<RichPluginTable>(node) {
            let c = ColorRgba::from_u32(row.color());
            let w = (ctx.rect.width as i32).min(16);
            let h = (ctx.rect.height as i32).min(16);
            for y in 0..h {
                for x in 0..w {
                    ctx.buffer.blend_pixel(x, y, c, ctx.opacity);
                }
            }
        }
    }
}

/// Extends warm retained-scene zero-alloc guard to cover `DrawText`, `DrawImage`,
/// `FillRoundedRectGradient`, `DrawShadow` (outer + inset), and `Plugin` ops together.
#[test]
fn warm_retained_scene_replay_covers_text_image_gradient_shadows_and_plugin_zero_allocs() {
    let _serial = SERIAL_TEST.lock().unwrap_or_else(|e| e.into_inner());
    let img_uri = "nve-test://compositor-zero-alloc-rich-img";
    ImageCache::shared()
        .insert_raw_rgba(img_uri, 8, 8, vec![180u8; 8 * 8 * 4])
        .expect("insert test image");

    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let mut backend = SkiaBackend::new();
    backend.register_table_system(RichPluginSystem);
    backend.set_viewport(
        SurfaceId::ROOT,
        Viewport {
            width: 320.0,
            height: 240.0,
            scale_factor: 1.0,
        },
    );

    let scroll = {
        let mut tx = store.tx();
        tx.edit::<Layout>(NodeId::ROOT)
            .width(Length::points(320.0))
            .height(Length::points(240.0));
        let scroll = tx.create();
        tx.edit::<Layout>(scroll)
            .width(Length::points(320.0))
            .height(Length::points(240.0))
            .flex_direction(FlexDirection::Column);
        tx.edit::<Scroll>(scroll);
        tx.edit::<Visual>(scroll).overflow_y(Overflow::Scroll);

        let card = tx.create();
        tx.edit::<Layout>(card)
            .width(Length::points(260.0))
            .height(Length::points(64.0))
            .flex_shrink(0.0);
        tx.edit::<Visual>(card)
            .border_radius(8.0)
            .border_width(2.0)
            .border_color(Rgba::rgb(148, 163, 184))
            .background_fill(Some(BackgroundFillRes(BackgroundFill::linear(
                90.0,
                vec![
                    GradientStop::new(0.0, ColorRgba::rgb(37, 99, 235)),
                    GradientStop::new(1.0, ColorRgba::rgb(16, 185, 129)),
                ],
            ))))
            .box_shadows(Some(BoxShadows::new(vec![
                BoxShadow::new(0.0, 4.0, 8.0, Rgba::new(0, 0, 0, 180)),
                BoxShadow::new(0.0, 3.0, 6.0, Rgba::new(15, 23, 42, 180)).inset(),
            ])));

        let img_node = tx.create();
        tx.edit::<Layout>(img_node)
            .width(Length::points(48.0))
            .height(Length::points(48.0))
            .flex_shrink(0.0);
        tx.edit::<Image>(img_node)
            .src(Some(ImageSrc::Url(img_uri.to_owned())))
            .fit(ImageFit::Cover);

        let text_node = tx.create();
        tx.edit::<Layout>(text_node)
            .width(Length::points(240.0))
            .height(Length::points(32.0))
            .flex_shrink(0.0);
        tx.edit::<Typography>(text_node)
            .font_size(14.0)
            .color(Rgba::WHITE);
        tx.edit::<TextContent>(text_node).text("Warm Retained Text");

        let plugin_node = tx.create();
        tx.edit::<Layout>(plugin_node)
            .width(Length::points(48.0))
            .height(Length::points(48.0))
            .flex_shrink(0.0);
        tx.edit::<RichPluginTable>(plugin_node)
            .color(ColorRgba::rgb(250, 204, 21).to_u32());

        let filler = tx.create();
        tx.edit::<Layout>(filler)
            .width(Length::points(260.0))
            .height(Length::points(300.0))
            .flex_shrink(0.0);

        tx.set_children(scroll, &[card, img_node, text_node, plugin_node, filler]);
        tx.set_children(NodeId::ROOT, &[scroll]);
        tx.commit_into(&mut changes);
        scroll
    };
    backend.commit(&store, &changes).expect("commit rich scene");

    // Software replay: warm up then assert 0 allocations across scroll + replay.
    let mut soft_buf = PixelBuffer::new(320, 240);
    backend.set_scroll_offset(&store, scroll, 0.0, 4.0);
    backend.rasterize_with_store(&store, &mut soft_buf, ColorRgba::BLACK);
    let soft_allocs = measure_allocs(|| {
        for step in 1..=4u32 {
            backend.set_scroll_offset(&store, scroll, 0.0, (step as f32) * 8.0);
            backend.rasterize_with_store(&store, &mut soft_buf, ColorRgba::BLACK);
        }
    });
    assert_eq!(
        soft_allocs, 0,
        "warm software replay of gradient+outer/inset shadows+image+text+plugin must perform 0 allocs"
    );

    #[cfg(feature = "skia")]
    {
        use nve_backend_skia::gpu::CpuSkiaSurface;
        let mut surface = CpuSkiaSurface::new(320, 240).expect("cpu skia surface");
        // Cold warm-up populates TextBlob, ImageSlotHandle, Plugin texture, and MaskFilter caches.
        {
            let (scene, reg, ev) = backend.scene_paint_parts_mut();
            surface
                .render_scene_cached_with_plugins(scene, 1.0, ColorRgba::BLACK, Some((&store, reg, ev)))
                .expect("cold rich skia scene");
        }
        surface.reset_cache_stats();
        let skia_allocs = measure_allocs(|| {
            for step in 1..=5u32 {
                backend.set_scroll_offset(&store, scroll, 0.0, (step as f32) * 6.0);
                let (scene, reg, ev) = backend.scene_paint_parts_mut();
                surface
                    .render_scene_cached_with_plugins(
                        scene,
                        1.0,
                        ColorRgba::BLACK,
                        Some((&store, reg, ev)),
                    )
                    .expect("warm rich skia scene");
            }
        });
        let st = surface.cache_stats();
        assert_eq!(st.text_blob_misses, 0);
        assert!(st.text_blob_hits >= 5);
        assert_eq!(st.image_misses, 0);
        assert_eq!(st.image_fast_hits, 5);
        assert_eq!(st.plugin_misses, 0);
        assert_eq!(st.plugin_hits, 5);
        assert_eq!(st.mask_filter_misses, 0);
        assert_eq!(st.mask_filter_hits, 10, "outer + inset shadows must hit blur cache");
        assert_eq!(
            skia_allocs, 0,
            "warm Skia CPU replay of gradient+outer/inset shadows+image+text+plugin must perform 0 Rust heap allocs"
        );
    }
}

#[cfg(feature = "skia")]
#[test]
fn skia_cpu_scene_warm_scroll_replay_zero_rust_allocs() {
    let _serial = SERIAL_TEST.lock().unwrap_or_else(|e| e.into_inner());
    use nve_backend_skia::gpu::{CpuSkiaSurface, GpuSurfaceBackend};

    let (store, _changes, mut backend, scroll, _sticky, _anim_node, _cards) =
        build_1000_node_rich_scroll_scene();
    let mut surface = CpuSkiaSurface::new(320, 240).expect("cpu skia surface");
    let mut before_buf = PixelBuffer::new(320, 240);
    let mut after_buf = PixelBuffer::new(320, 240);

    // Warm up one scroll + Skia render.
    backend.set_scroll_offset(&store, scroll, 0.0, 24.0);
    surface
        .render_scene_cached_with_plugins(backend.scene(), 1.0, ColorRgba::BLACK, None)
        .expect("warm skia scene");
    surface
        .copy_to_pixel_buffer(&mut before_buf)
        .expect("readback before");
    let fast_before = backend.scene().stats().scroll_fast_paths;

    let allocs = measure_allocs(|| {
        for step in 1..=5u32 {
            backend.set_scroll_offset(&store, scroll, 0.0, (step as f32) * 40.0);
            surface
                .render_scene_cached_with_plugins(backend.scene(), 1.0, ColorRgba::BLACK, None)
                .expect("warm skia scene render");
        }
    });

    surface
        .copy_to_pixel_buffer(&mut after_buf)
        .expect("readback after");
    assert_eq!(backend.scene().stats().scroll_fast_paths, fast_before + 5);
    assert_ne!(
        before_buf.pixels(),
        after_buf.pixels(),
        "Skia CPU scrolled output must actually change"
    );
    assert_eq!(
        allocs, 0,
        "warm Skia CPU CompositorScene scroll replay must perform 0 Rust heap allocations, got {allocs}"
    );
}
