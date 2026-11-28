//! Pass-3 dual-review regression guards for Commit 2 / Slice 4B (`packages/backends/skia`):
//! - Per-slot atomic `ImageSlotHandle` fast path survives sibling inserts & oversized uncached draws,
//!   and emit is strictly peek-only (zero decode during commit)
//! - Partial multi-run font failure aborts blob construction instead of dropping secondary runs
//! - Heap-backed `Binary` typefaces enforce `MAX_COPIED_TYPEFACE_BYTES` while mmap faces charge `0`
//! - Dynamic-table (`table_desc == None`) resource-word invalidation + hash-set path (`> 4` resources)
//! - Oversized transient plugin draw shrinks `plugin_buf` / `plugin_rgba` scratch buffers
//! - Focused `TextInput` caret snapping clamps inside `text_rect` clip at right edge & extreme scale

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use nve_abi::{ColorRgba, EventBatchBuf, Key64, SurfaceId};
use nve_engine::{LayoutRect, StoreBackend, Viewport};
use nve_store::wire::{Op, WireDecoder, MAGIC, VERSION};
use nve_store::{
    ChangeSet, FlatResource, Length, NodeId, NodeStore, NveTable, Res, Rgba, TableSlot,
    TableStorage, ValidatedWords, MAX_STATIC_TABLE_SLOTS,
};
use nve_tables::tables::{Image, ImageFit, ImageSrc, Layout, TextInput, Typography};

use super::cache_parity_tests::{count_lit_ink, lock_shared_image_cache, ParityPlugin};
use super::super::*;
use crate::image_cache::{encode_rgb_png, ImageCache};
use crate::raster::PixelBuffer;
use crate::table_system::{SkiaSystemPaintCtx, SkiaTableRegistry, SkiaTableSystem};
use crate::text::{CosmicTextEngine, TextAlign};
use crate::{DisplayList, PaintOp, SkiaBackend};

fn encode_base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut b64 = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let triple = ((chunk[0] as u32) << 16)
            | ((*chunk.get(1).unwrap_or(&0) as u32) << 8)
            | (*chunk.get(2).unwrap_or(&0) as u32);
        b64.push(TABLE[((triple >> 18) & 63) as usize] as char);
        b64.push(TABLE[((triple >> 12) & 63) as usize] as char);
        b64.push(if chunk.len() > 1 { TABLE[((triple >> 6) & 63) as usize] as char } else { '=' });
        b64.push(if chunk.len() > 2 { TABLE[(triple & 63) as usize] as char } else { '=' });
    }
    b64
}

fn make_png_data_uri(r: u8, g: u8, b: u8) -> String {
    let png = encode_rgb_png(4, 4, &[r, g, b].repeat(16)).expect("encode 4x4 PNG");
    format!("data:image/png;base64,{}", encode_base64(&png))
}

#[test]
fn multi_image_fast_path_survives_sibling_inserts_and_oversized_uncached_draws() {
    let _img_guard = lock_shared_image_cache();
    let cache = ImageCache::shared();
    cache.clear();

    let uri_a = make_png_data_uri(240, 30, 30);
    let uri_b = make_png_data_uri(30, 220, 60);
    let uri_c = make_png_data_uri(30, 80, 240);

    // 1. Commit a store containing 2 uncached PNG data URIs: peek-only emit MUST NOT decode them.
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let (node_a, node_b) = {
        let mut tx = store.tx();
        let a = tx.create();
        let b = tx.create();
        tx.set_children(NodeId::ROOT, &[a, b]);
        tx.edit::<Layout>(a)
            .width(Length::points(32.0))
            .height(Length::points(32.0));
        tx.edit::<Image>(a)
            .src(Some(ImageSrc::Url(uri_a.clone())))
            .fit(ImageFit::Fill);
        tx.edit::<Layout>(b)
            .width(Length::points(32.0))
            .height(Length::points(32.0));
        tx.edit::<Image>(b)
            .src(Some(ImageSrc::Url(uri_b.clone())))
            .fit(ImageFit::Fill);
        tx.commit_into(&mut changes);
        (a, b)
    };
    let _ = (node_a, node_b);

    let mut backend = SkiaBackend::new();
    backend.set_viewport(SurfaceId::ROOT, Viewport { width: 120.0, height: 80.0, scale_factor: 1.0 });
    backend.commit(&store, &changes).unwrap();
    assert!(
        cache.resolve_cached_handle(&uri_a).is_none()
            && cache.resolve_cached_handle(&uri_b).is_none(),
        "peek-only emit at commit time must not decode uncached PNG URIs"
    );

    // 2. First draw of retained `CompositorScene` lazily decodes & uploads both images and
    //    backfills their `Cell<Option<ImageSlotHandle>>` slots in place!
    let mut surface = CpuSkiaSurface::new(120, 80).unwrap();
    surface.render_scene_cached_with_plugins(backend.scene(), 1.0, ColorRgba::BLACK, None).unwrap();
    assert!(cache.resolve_cached_handle(&uri_a).is_some() && cache.resolve_cached_handle(&uri_b).is_some());
    assert_eq!(surface.cache_stats().image_misses, 2);

    // 3. Insert a 3rd sibling image AFTER frame 1 minted handles: rendering the EXACT SAME
    //    `backend.scene()` directly on frame 2 MUST hit the lock-free fast path for BOTH images!
    let _ = cache.get_or_decode(&uri_c).expect("decode sibling C");
    surface.reset_cache_stats();
    surface.render_scene_cached_with_plugins(backend.scene(), 1.0, ColorRgba::BLACK, None).unwrap();
    let st = surface.cache_stats();
    assert_eq!(st.image_fast_hits, 2, "frame 2 must hit lock-free per-slot fast path");
    assert_eq!(st.image_misses, 0);

    // 4. Evict/clear global ImageCache -> frame 3 misses & self-heals `image_handles`, frame 4
    //    is immediately back on the lock-free fast path without re-emitting chunks!
    cache.clear();
    surface.reset_cache_stats();
    surface.render_scene_cached_with_plugins(backend.scene(), 1.0, ColorRgba::BLACK, None).unwrap();
    assert_eq!((surface.cache_stats().image_fast_hits, surface.cache_stats().image_misses), (0, 2));

    surface.reset_cache_stats();
    surface.render_scene_cached_with_plugins(backend.scene(), 1.0, ColorRgba::BLACK, None).unwrap();
    assert_eq!(surface.cache_stats().image_fast_hits, 2, "scene must self-heal after eviction");
    assert_eq!(surface.cache_stats().image_misses, 0);

    // 5. Bounded local ImageCache: real oversized PNG URI, overwrite-with-oversized, CLOCK
    //    second-chance fast-path LRU retention, LRU eviction + slot reuse, and `clear()`.
    let small = ImageCache::new(3, 64);
    small.insert_raw_rgba("k1", 2, 2, vec![1u8; 16]).unwrap();
    small.insert_raw_rgba("k2", 2, 2, vec![2u8; 16]).unwrap();
    small.insert_raw_rgba("k3", 2, 2, vec![3u8; 16]).unwrap();
    let s1 = small.resolve_cached_handle("k1").unwrap();
    let s2 = small.resolve_cached_handle("k2").unwrap();
    let s3 = small.resolve_cached_handle("k3").unwrap();

    // Real 8x8 PNG URI (256 RGBA bytes > 64-byte budget) driven through get_or_decode_with_slot.
    let huge_png = encode_rgb_png(8, 8, &[9u8; 8 * 8 * 3]).expect("encode huge png");
    let huge_uri = format!("data:image/png;base64,{}", encode_base64(&huge_png));
    let (huge_slot, huge_dec) = small
        .get_or_decode_with_slot(&huge_uri)
        .expect("decode oversized png");
    assert!(huge_slot.is_none(), "oversized image must return slot=None");
    assert_eq!((huge_dec.width, huge_dec.height), (8, 8));
    assert_eq!(small.resolve_cached_handle("k1"), Some(s1));
    assert_eq!(small.resolve_cached_handle("k2"), Some(s2));
    assert_eq!(small.resolve_cached_handle("k3"), Some(s3));
    // Verify SkiaRendererCaches accepts slot=None oversized upload without evicting cached slots.
    let mut local_caches = crate::gpu::renderer_cache::SkiaRendererCaches::new();
    let uploaded_oversized = local_caches.get_or_upload_image_for_slot(
        None,
        &huge_dec,
        0,
        #[cfg(any(feature = "skia-metal", feature = "skia-vulkan", feature = "skia-d3d"))]
        None,
    );
    assert!(uploaded_oversized.is_some());
    assert_eq!(local_caches.stats().image_entries, 0);

    // CLOCK second-chance LRU: k1 is at the LRU tail, touch k1 ONLY via lock-free `is_handle_live`,
    // then insert k4 -> k1 MUST survive while cold k2 is evicted!
    assert!(small.is_handle_live(s1));
    small.insert_raw_rgba("k4", 2, 2, vec![4u8; 16]).unwrap();
    assert!(
        small.is_handle_live(s1),
        "hot fast-path image (k1) must survive eviction via CLOCK second-chance"
    );
    assert!(!small.is_handle_live(s2), "cold LRU image (k2) must be evicted");
    assert!(small.is_handle_live(s3));

    // Overwriting existing key k3 with an oversized image invalidates s3 while keeping s1 live.
    small.insert_raw_rgba("k3", 8, 8, vec![5u8; 256]).unwrap();
    assert!(!small.is_handle_live(s3), "oversized overwrite of k3 must invalidate s3");
    assert!(small.is_handle_live(s1));

    // `clear()` invalidates all remaining live handles.
    small.clear();
    assert!(!small.is_handle_live(s1));
}

#[test]
fn partial_multi_run_font_failure_aborts_blob_and_falls_back_to_bitmap() {
    let engine = Arc::new(CosmicTextEngine::new());
    if !engine.has_system_fonts() {
        assert!(
            std::env::var_os("NVE_ALLOW_NO_SYSTEM_FONTS").is_some(),
            "system fonts unavailable (set NVE_ALLOW_NO_SYSTEM_FONTS=1 only in fontless containers)"
        );
        return;
    }

    // Find a multi-font sample whose runs resolve to >= 2 distinct fontdb::IDs.
    let mut runs = crate::text::SkiaShapedRuns::default();
    let mut found = None;
    for sample in ["Hello 🚀", "Hello مرحبا", "Hello 日本語"] {
        engine.shape_for_skia_runs(sample, 18.0, 400, TextAlign::Left, None, 220.0, &mut runs);
        if let Some(first) = runs.glyphs.first().map(|g| g.font_id)
            && let Some(second) = runs.glyphs.iter().map(|g| g.font_id).find(|&id| id != first)
        {
            found = Some((sample, second));
            break;
        }
    }
    let Some((sample, second_face)) = found else {
        panic!("expected at least one multi-face string among emoji/RTL/CJK");
    };

    // Fail ONLY the secondary run's face: TextBlob construction MUST abort and return None
    // (never caching a partial blob that silently dropped the secondary run).
    *engine.fail_only_face_id.lock().unwrap() = Some(second_face);
    let mut surface = CpuSkiaSurface::with_text_engine(240, 60, 1.0, Arc::clone(&engine)).unwrap();
    assert!(
        surface
            .renderer()
            .build_cached_text_blob(sample, 18.0, 400, TextAlign::Left, None, 220.0)
            .is_none(),
        "partial multi-run font failure must return None rather than a partial TextBlob"
    );

    let dl = DisplayList {
        ops: vec![PaintOp::DrawText {
            rect: LayoutRect { x: 8.0, y: 8.0, width: 220.0, height: 40.0 },
            text: sample.to_owned(),
            font_size: 18.0,
            font_weight: 400,
            text_align: TextAlign::Left,
            color: ColorRgba::WHITE,
        }],
    };
    surface.render_cached_with_plugins(&dl, 1.0, ColorRgba::BLACK, None).unwrap();
    let mut out = PixelBuffer::new(240, 60);
    surface.copy_to_pixel_buffer(&mut out).unwrap();
    assert!(count_lit_ink(&out, 8, 8, 228, 48) > 3_000, "bitmap fallback must render text");
}

#[test]
fn binary_typefaces_respect_heap_byte_cap_while_mmap_faces_charge_zero() {
    let engine = CosmicTextEngine::shared();
    if !engine.has_system_fonts() {
        assert!(
            std::env::var_os("NVE_ALLOW_NO_SYSTEM_FONTS").is_some(),
            "system fonts unavailable (set NVE_ALLOW_NO_SYSTEM_FONTS=1 only in fontless containers)"
        );
        return;
    }

    let mut runs = crate::text::SkiaShapedRuns::default();
    assert!(engine.shape_for_skia_runs("Cap test", 16.0, 400, TextAlign::Left, None, 200.0, &mut runs));
    let face_a = runs.glyphs[0].font_id;
    let raw_bytes = engine
        .with_face_bytes(face_a, |b, _| b.to_vec())
        .expect("raw font bytes");
    let face_len = raw_bytes.len();
    assert!(face_len > 64);

    // 1. Two in-memory `Binary` faces in a cache capped to `face_len + 32` bytes must evict the
    //    first face when the second is loaded (strictly bounded by `max_bytes`).
    let mut bin_db = cosmic_text::fontdb::Database::new();
    bin_db.load_font_data(raw_bytes.clone());
    bin_db.load_font_data(raw_bytes);
    let ids: Vec<_> = bin_db.faces().map(|f| f.id).collect();
    assert!(ids.len() >= 2);
    let bin_fs = cosmic_text::FontSystem::new_with_locale_and_db("en-US".to_string(), bin_db);
    let bin_engine = Arc::new(CosmicTextEngine::from_font_system(bin_fs));
    let bin_renderer = SkiaCanvasRenderer::with_text_engine(bin_engine);
    bin_renderer.set_base_typefaces_byte_cap_for_test(face_len + 32);

    assert!(bin_renderer.resolve_typeface_for_test(ids[0], 400).is_some());
    assert!(bin_renderer.resolve_typeface_for_test(ids[1], 400).is_some());
    let st = bin_renderer.cache_stats();
    assert_eq!(st.typeface_entries, 1, "Binary faces must respect heap byte cap");
    assert_eq!(st.typeface_copied_entries, 1);
    assert!(st.evictions >= 1);

    // 1b. Oversized Binary face (> byte cap) is still admitted (charged `cap`) instead of rejected.
    bin_renderer.set_base_typefaces_byte_cap_for_test(128);
    assert!(bin_renderer.resolve_typeface_for_test(ids[0], 400).is_some());
    assert_eq!(bin_renderer.cache_stats().typeface_entries, 1);
    bin_renderer.reset_cache_stats();
    assert!(bin_renderer.resolve_typeface_for_test(ids[0], 400).is_some());
    assert_eq!(bin_renderer.cache_stats().typeface_hits, 1);

    // 2. Mmap-backed system faces charge `0` against the heap cap even when the cap is tiny (128 B),
    //    while `stats.typeface_bytes` still reports their true total file bytes.
    let mmap_renderer = SkiaCanvasRenderer::new();
    mmap_renderer.set_base_typefaces_byte_cap_for_test(128);
    let second_sys_face = engine
        .all_face_ids()
        .into_iter()
        .find(|&fid| fid != face_a && mmap_renderer.resolve_typeface_for_test(fid, 400).is_some())
        .expect("second decodable system face");
    assert!(mmap_renderer.resolve_typeface_for_test(face_a, 400).is_some());
    let mst = mmap_renderer.cache_stats();
    assert!(
        mst.typeface_entries >= 2,
        "mmap faces must charge 0 against heap cap (not evicted at 128 B cap); second={second_sys_face:?}"
    );
    assert!(mst.typeface_bytes >= face_len);
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DynColorPayload(u32);

impl FlatResource for DynColorPayload {
    const KIND: Key64 = Key64::of("nve.test.pass3.dyn_color");
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
#[nve_table(name = "nve.test.pass3_res_holder", storage = "sparse")]
#[allow(dead_code)]
struct DynResHolder {
    #[effect(paint)]
    pub payload: Res<DynColorPayload>,
}

struct DynSlotSystem {
    slot: TableSlot,
    calls: Arc<AtomicU32>,
}

impl SkiaTableSystem for DynSlotSystem {
    fn slot(&self) -> TableSlot {
        self.slot
    }

    fn paint_node(&self, ctx: &mut SkiaSystemPaintCtx<'_>, store: &NodeStore, node: NodeId) {
        self.calls.fetch_add(1, Ordering::Relaxed);
        let Some(TableStorage { rows, owners, row_count, .. }) = store.table_storage(self.slot) else {
            return;
        };
        let Some(pos) = owners[..row_count].iter().position(|&o| o == node) else {
            return;
        };
        let rid = nve_store::ResourceId(rows[pos]);
        let rgba = store
            .resource::<DynColorPayload>(rid)
            .unwrap_or(ColorRgba::WHITE.to_u32());
        let c = ColorRgba::from_u32(rgba);
        for y in 0..(ctx.rect.height.round() as i32).clamp(1, 16) {
            for x in 0..(ctx.rect.width.round() as i32).clamp(1, 16) {
                ctx.buffer.blend_pixel(x, y, c, 1.0);
            }
        }
    }
}

fn build_single_op_wire_frame(op: u8, table: u8, count: u16, body: &[u8]) -> Vec<u8> {
    assert!(body.len().is_multiple_of(4));
    let mut buf = Vec::new();
    buf.extend_from_slice(&MAGIC.to_le_bytes());
    buf.extend_from_slice(&VERSION.to_le_bytes());
    buf.extend_from_slice(&0u16.to_le_bytes());
    buf.extend_from_slice(&1u64.to_le_bytes());
    buf.extend_from_slice(&1u32.to_le_bytes());
    let total_words = (6 + 2 + body.len() / 4) as u32;
    buf.extend_from_slice(&total_words.to_le_bytes());
    let head = (op as u32) | ((table as u32) << 8) | ((count as u32) << 16);
    buf.extend_from_slice(&head.to_le_bytes());
    buf.extend_from_slice(&((body.len() / 4) as u32).to_le_bytes());
    buf.extend_from_slice(body);
    buf
}

#[test]
fn dynamic_table_resource_word_invalidation_repaints_plugin() {
    let mut replica = NodeStore::new();
    let mut dec = WireDecoder::default();
    let mut changes = ChangeSet::default();

    // Declare a wire-only 1-word sparse dynamic table with bit 0 marked as a resource word.
    let dyn_key = Key64::of("nve.test.pass3.dyn_plugin_table");
    let wire_slot = 0u8;
    let mut decl_body = Vec::new();
    decl_body.extend_from_slice(&(dyn_key.0 as u32).to_le_bytes());
    decl_body.extend_from_slice(&((dyn_key.0 >> 32) as u32).to_le_bytes());
    decl_body.extend_from_slice(&0x5555u32.to_le_bytes());
    decl_body.extend_from_slice(&0u32.to_le_bytes());
    // wire_slot=0, sparse(1<<8), flags=0, words=1(1<<24)
    let packed = (wire_slot as u32) | (1u32 << 8) | (1u32 << 24);
    decl_body.extend_from_slice(&packed.to_le_bytes());
    decl_body.extend_from_slice(&0u32.to_le_bytes()); // str_words
    decl_body.extend_from_slice(&1u32.to_le_bytes()); // resource_words (word 0)
    decl_body.extend_from_slice(&0u32.to_le_bytes()); // default[0]
    decl_body.extend_from_slice(&[1, 0, 0, 0]); // word_effects (paint)
    let name = b"nve.test.pass3.dyn_plugin";
    decl_body.extend_from_slice(&(name.len() as u32).to_le_bytes());
    decl_body.extend_from_slice(name);
    while !decl_body.len().is_multiple_of(4) {
        decl_body.push(0);
    }
    let decl_frame = build_single_op_wire_frame(Op::DeclareTables as u8, 0, 1, &decl_body);
    {
        let mut tx = replica.tx();
        dec.apply(&mut tx, &decl_frame).expect("declare dynamic table");
        tx.commit_into(&mut changes);
    }
    let dyn_slot = replica
        .table_slots()
        .find(|&s| (s.0 as usize) >= MAX_STATIC_TABLE_SLOTS)
        .expect("allocated dynamic slot");
    assert!(
        replica.table_desc(dyn_slot).is_none(),
        "wire-declared dynamic table must have table_desc == None"
    );

    // Create a holder node owning an ephemeral resource and attach `dyn_slot` pointing to that ResourceId.
    let (holder, dyn_node, extra_holders) = {
        let mut tx = replica.tx();
        let h = tx.create();
        let d = tx.create();
        let extras: Vec<NodeId> = (0..5).map(|_| tx.create()).collect();
        let mut children = vec![h, d];
        children.extend_from_slice(&extras);
        tx.set_children(NodeId::ROOT, &children);
        tx.edit::<Layout>(d)
            .width(Length::points(32.0))
            .height(Length::points(32.0));
        tx.edit::<DynResHolder>(h)
            .payload_ephemeral(DynColorPayload(ColorRgba::rgb(250, 40, 40).to_u32()));
        tx.commit_into(&mut changes);
        (h, d, extras)
    };
    let calls = Arc::new(AtomicU32::new(0));
    let mut backend = SkiaBackend::new();
    backend.register_table_system(DynSlotSystem {
        slot: dyn_slot,
        calls: Arc::clone(&calls),
    });
    backend.set_viewport(
        SurfaceId::ROOT,
        Viewport {
            width: 64.0,
            height: 64.0,
            scale_factor: 1.0,
        },
    );
    backend.commit(&replica, &changes).unwrap();

    let rid = replica.row::<DynResHolder>(holder).payload();
    let mut patch_body = Vec::new();
    patch_body.extend_from_slice(&dyn_node.raw().to_le_bytes());
    patch_body.extend_from_slice(&1u32.to_le_bytes()); // mask = word 0
    patch_body.extend_from_slice(&rid.0.to_le_bytes());
    let patch_frame = build_single_op_wire_frame(Op::Patch as u8, wire_slot, 1, &patch_body);
    {
        let mut tx = replica.tx();
        dec.apply(&mut tx, &patch_frame).expect("patch dynamic row");
        tx.commit_into(&mut changes);
    }
    backend.commit(&replica, &changes).unwrap();

    let mut surface = CpuSkiaSurface::new(64, 64).unwrap();
    let mut buf = PixelBuffer::new(64, 64);
    {
        let (scene, reg, ev) = backend.scene_paint_parts_mut();
        surface
            .render_scene_cached_with_plugins(scene, 1.0, ColorRgba::BLACK, Some((&replica, reg, ev)))
            .unwrap();
    }
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    surface.copy_to_pixel_buffer(&mut buf).unwrap();
    assert_eq!(buf.get_rgb(4, 4), Some((250, 40, 40)));

    // Mutate `rid` in-place via `Op::DefineResources` plus 5 extra resources (> 4 changed
    // resources exercises the HashSet path) WITHOUT modifying `dyn_node`'s row: dynamic-table
    // scan MUST invalidate and repaint `dyn_node`.
    let mut def_body = Vec::new();
    def_body.extend_from_slice(&rid.0.to_le_bytes());
    def_body.extend_from_slice(&(DynColorPayload::KIND.0 as u32).to_le_bytes());
    def_body.extend_from_slice(&((DynColorPayload::KIND.0 >> 32) as u32).to_le_bytes());
    def_body.extend_from_slice(&1u32.to_le_bytes()); // ephemeral = true
    def_body.extend_from_slice(&0u32.to_le_bytes()); // child_count = 0
    def_body.extend_from_slice(&4u32.to_le_bytes()); // len = 4
    def_body.extend_from_slice(&ColorRgba::rgb(30, 230, 120).to_u32().to_le_bytes());
    let def_frame = build_single_op_wire_frame(Op::DefineResources as u8, 0, 1, &def_body);
    {
        let mut tx = replica.tx();
        dec.apply(&mut tx, &def_frame).expect("redefine resource in place");
        for (i, &eh) in extra_holders.iter().enumerate() {
            tx.edit::<DynResHolder>(eh)
                .payload_ephemeral(DynColorPayload(i as u32 + 10));
        }
        tx.commit_into(&mut changes);
    }
    let _ = holder;
    assert!(changes.resources_defined.len() >= 6);
    backend.commit(&replica, &changes).unwrap();
    {
        let (scene, reg, ev) = backend.scene_paint_parts_mut();
        surface
            .render_scene_cached_with_plugins(scene, 1.0, ColorRgba::BLACK, Some((&replica, reg, ev)))
            .unwrap();
    }
    assert_eq!(
        calls.load(Ordering::Relaxed),
        2,
        "dynamic-table resource change must invalidate plugin cache"
    );
    surface.copy_to_pixel_buffer(&mut buf).unwrap();
    assert_eq!(buf.get_rgb(4, 4), Some((30, 230, 120)));
}

#[test]
fn oversized_plugin_draw_shrinks_scratch_buffers_and_pixel_buffer_shrink_works() {
    // 1. Direct PixelBuffer::shrink_if_capacity_exceeds verification.
    let mut pb = PixelBuffer::new_transparent_rgba(1200, 1000);
    assert!(pb.pixel_capacity() >= 1_200_000);
    pb.shrink_if_capacity_exceeds(1024 * 1024);
    assert_eq!(pb.pixel_capacity(), 0);
    assert_eq!((pb.width(), pb.height()), (0, 0));

    // 2. Transient oversized (> 1 Mi pixels) plugin draw in CpuSkiaSurface releases both scratch
    //    buffers immediately after uploading to Skia.
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let node = {
        let mut tx = store.tx();
        let n = tx.create();
        tx.set_children(NodeId::ROOT, &[n]);
        tx.edit::<ParityPlugin>(n)
            .color_rgba(ColorRgba::rgb(90, 180, 250).to_u32());
        tx.commit_into(&mut changes);
        n
    };
    let calls = Arc::new(AtomicU32::new(0));
    let mut registry = SkiaTableRegistry::new();
    registry.register(super::cache_parity_tests::CountingPluginSystem { calls });
    let mut events = EventBatchBuf::new();

    let mut surface = CpuSkiaSurface::new(128, 128).unwrap();
    let dl = DisplayList {
        ops: vec![PaintOp::Plugin {
            node_id: node,
            rect: LayoutRect { x: 0.0, y: 0.0, width: 1100.0, height: 1100.0 },
            inherited_text_color: Rgba::WHITE,
            plugin_epoch: 1,
        }],
    };
    surface.render_cached_with_plugins(&dl, 1.0, ColorRgba::BLACK, Some((&store, &registry, &mut events))).unwrap();
    assert_eq!(surface.renderer().plugin_scratch_capacities(), (0, 0), "oversized scratch must shrink");
}

#[test]
fn caret_snapping_clamps_inside_text_clip_at_right_edge_and_extreme_scale() {
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let input = {
        let mut tx = store.tx();
        let n = tx.create();
        tx.set_children(NodeId::ROOT, &[n]);
        tx.edit::<Layout>(n).width(Length::points(40.0)).height(Length::points(32.0));
        tx.edit::<Typography>(n).font_size(14.0).color(Rgba::WHITE);
        tx.edit::<TextInput>(n).value("A very long input string that overflows the 40px box");
        tx.commit_into(&mut changes);
        n
    };

    let mut backend = SkiaBackend::new();
    for scale in [1.0f32, 1.25, 1.5, 1.75, 1000.0] {
        backend.set_viewport(SurfaceId::ROOT, Viewport { width: 80.0, height: 60.0, scale_factor: scale });
        backend.commit(&store, &changes).unwrap();
        backend.set_focused_node(&store, Some(input));
        let box_rect = backend.layout_tree().layout_of(input).expect("input layout rect");
        let caret = backend.ime_cursor_rect(&store).expect("focused caret rect");
        assert!(caret.x.is_finite() && caret.y.is_finite() && caret.width.is_finite() && caret.height.is_finite());
        assert!(
            caret.x + caret.width <= box_rect.x + box_rect.width + 1e-4,
            "scale={scale}: caret ({}, w={}) overhangs right edge {}",
            caret.x, caret.width, box_rect.x + box_rect.width
        );
        let dl = backend.materialize_display_list();
        let scene_caret = dl.ops.iter().rev().find_map(|op| match op {
            PaintOp::FillRoundedRect { rect, color, .. } if *color == crate::FOCUS_RING_COLOR => Some(*rect),
            _ => None,
        }).expect("materialized scene caret");
        assert!(
            scene_caret.x + scene_caret.width <= box_rect.x + box_rect.width + 1e-4,
            "scale={scale}: scene caret overhangs right edge"
        );
    }

    let (padded, narrow) = {
        let mut tx = store.tx();
        let p = tx.create();
        tx.edit::<Layout>(p).width(Length::points(48.0)).height(Length::points(32.0))
            .padding_left(Length::points(8.0)).padding_right(Length::points(8.0));
        tx.edit::<Typography>(p).font_size(14.0).color(Rgba::WHITE);
        tx.edit::<TextInput>(p).value("Long overflowing padded input text");
        let n = tx.create();
        tx.edit::<Layout>(n).width(Length::points(1.0)).height(Length::points(24.0));
        tx.edit::<Typography>(n).font_size(14.0).color(Rgba::WHITE);
        tx.edit::<TextInput>(n).value("X");
        tx.set_children(NodeId::ROOT, &[input, p, n]);
        tx.commit_into(&mut changes);
        (p, n)
    };
    backend.commit(&store, &changes).unwrap();
    for node in [padded, narrow] {
        backend.set_focused_node(&store, Some(node));
        let caret = backend.ime_cursor_rect(&store).expect("caret");
        assert!(caret.x.is_finite() && caret.width.is_finite() && caret.height.is_finite());
    }
}
