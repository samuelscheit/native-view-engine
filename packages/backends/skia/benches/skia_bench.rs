//! Criterion benchmarks for `nve-backend-skia`: `SkiaBackend::commit` retained scene updates,
//! zero-C++ software `PixelBuffer` rasterization, `data:image/png;base64,...` decoding,
//! `SlabLru` O(1) hit/miss/churn microbenchmarks, and cached Skia CPU warm frames.

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::hint::black_box;

    use criterion::{criterion_group, Criterion};
    use nve_abi::{ColorRgba, SurfaceId};
    use nve_backend_skia::raster::{rasterize_display_list, PixelBuffer};
    use nve_backend_skia::slab_lru::{IdentityU64BuildHasher, SlabLru};
    use nve_backend_skia::{decode_image_uri, SkiaBackend};
    use nve_engine::{StoreBackend, Viewport};
    use nve_store::{ChangeSet, Length, NodeId, NodeStore, Rgba};
    use nve_tables::tables::{Layout, TextContent, Visual};

    const SAMPLE_PNG_DATA_URI: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAgAAAAICAYAAADED76LAAAAJElEQVR42mOQ07T+jw8zgAgerzysGEWBxd4fKHggFOB1JD4MAKjRfBnTmv6zAAAAAElFTkSuQmCC";

    fn bench_skia_backend(c: &mut Criterion) {
        let mut store = NodeStore::new();
        let mut changes = ChangeSet::default();
        let ids: Vec<NodeId> = {
            let mut tx = store.tx();
            let nodes: Vec<NodeId> = (0..64)
                .map(|i| {
                    let mut nm = tx.spawn();
                    nm.edit::<Layout>()
                        .width(Length::points(180.0))
                        .height(Length::points(28.0));
                    nm.edit::<Visual>()
                        .background(Rgba::rgb(37, 99, 235))
                        .border_radius(6.0);
                    if i % 4 == 0 {
                        nm.edit::<TextContent>().text("Skia Card");
                    }
                    nm.id()
                })
                .collect();
            tx.set_children(NodeId::ROOT, &nodes);
            tx.commit_into(&mut changes);
            nodes
        };

        let mut backend = SkiaBackend::new();
        backend.set_viewport(
            SurfaceId::ROOT,
            Viewport {
                width: 320.0,
                height: 240.0,
                scale_factor: 1.0,
            },
        );
        backend
            .commit(&store, &changes)
            .expect("initial skia bench commit");
        // Snapshot the opaque 64-card frame BEFORE the opacity-mutating commit bench so the warm
        // Skia CPU frame benchmark measures realistic opaque cards (not 64 full-screen saveLayers).
        let opaque_list = backend.materialize_display_list().clone();

        let mut tick = 0u32;
        c.bench_function("skia_commit_and_display_list_64_nodes", |b| {
            b.iter(|| {
                tick = tick.wrapping_add(1);
                let mut tx = store.tx();
                tx.edit::<Visual>(ids[(tick as usize) % ids.len()])
                    .opacity(((tick & 255) as f32) / 255.0);
                tx.commit_into(&mut changes);
                backend
                    .commit(&store, &changes)
                    .expect("incremental skia commit");
                black_box(backend.scene().stats().composite_fast_paths)
            });
        });

        #[cfg(feature = "skia")]
        let opacity_layers_list = backend.materialize_display_list().clone();
        let mut fb = PixelBuffer::new(320, 240);
        c.bench_function("skia_software_rasterize_320x240", |b| {
            b.iter(|| {
                rasterize_display_list(black_box(&opaque_list), &mut fb, 1.0, ColorRgba::BLACK);
                black_box(fb.pixels().len())
            });
        });

        c.bench_function("skia_decode_png_data_uri", |b| {
            b.iter(|| {
                let img = decode_image_uri(black_box(SAMPLE_PNG_DATA_URI))
                    .expect("valid bench PNG data URI");
                black_box(img.rgba.len())
            });
        });

        // SlabLru O(1) hit, miss+evict, and 10k churn microbenchmarks.
        let mut lru_hit: SlabLru<u64, u64, IdentityU64BuildHasher> =
            SlabLru::with_initial_capacity_and_hasher(1024, 1024 * 64, 1024, IdentityU64BuildHasher);
        for i in 0..1024u64 {
            lru_hit.insert(i, i.wrapping_mul(31), 64);
        }
        let mut k = 0u64;
        c.bench_function("slab_lru_hit", |b| {
            b.iter(|| {
                k = (k + 1) & 1023;
                black_box(lru_hit.get_mut(&black_box(k)).copied())
            });
        });

        let mut lru_evict: SlabLru<u64, u64, IdentityU64BuildHasher> =
            SlabLru::with_initial_capacity_and_hasher(256, 256 * 64, 256, IdentityU64BuildHasher);
        for i in 0..256u64 {
            lru_evict.insert(i, i, 64);
        }
        let mut next_key = 256u64;
        c.bench_function("slab_lru_miss_evict", |b| {
            b.iter(|| {
                next_key = next_key.wrapping_add(1);
                lru_evict.insert(black_box(next_key), next_key, 64);
                black_box(lru_evict.len())
            });
        });

        c.bench_function("slab_lru_10k_churn", |b| {
            b.iter(|| {
                let mut lru: SlabLru<u64, u64, IdentityU64BuildHasher> =
                    SlabLru::with_initial_capacity_and_hasher(
                        2048,
                        2048 * 64,
                        2048,
                        IdentityU64BuildHasher,
                    );
                for i in 0..10_000u64 {
                    lru.insert(black_box(i), i, 64);
                }
                black_box(lru.evictions())
            });
        });

        #[cfg(feature = "skia")]
        {
            use nve_backend_skia::{CpuSkiaSurface, DisplayList, ImageCache, PaintOp};
            use nve_engine::LayoutRect;
            use nve_tables::tables::ImageFit;

            let mut cpu = CpuSkiaSurface::new(320, 240).expect("bench cpu skia surface");
            cpu.render_cached_with_plugins(&opaque_list, 1.0, ColorRgba::BLACK, None)
                .expect("warm bench cpu frame");
            c.bench_function("skia_cpu_cached_warm_frame_320x240", |b| {
                b.iter(|| {
                    cpu.render_cached_with_plugins(
                        black_box(&opaque_list),
                        1.0,
                        ColorRgba::BLACK,
                        None,
                    )
                    .expect("bench warm render");
                    black_box(cpu.cache_stats().text_blob_hits)
                });
            });

            // Also benchmark the 64-layer SaveOpacity stress list to track offscreen layer cost.
            cpu.render_cached_with_plugins(&opacity_layers_list, 1.0, ColorRgba::BLACK, None)
                .expect("warm opacity layers frame");
            c.bench_function("skia_cpu_64_opacity_layers_320x240", |b| {
                b.iter(|| {
                    cpu.render_cached_with_plugins(
                        black_box(&opacity_layers_list),
                        1.0,
                        ColorRgba::BLACK,
                        None,
                    )
                    .expect("bench opacity layers render");
                    black_box(cpu.cache_stats().text_blob_hits)
                });
            });

            // Prove warm DrawImage cost is independent of URI length (16 B vs 16 KiB data URI),
            // even when each handle is resolved BEFORE later sibling images are inserted!
            let short_uri = "nve-b://short16!".to_owned();
            let long_uri = format!("nve-b://{}", "x".repeat(16 * 1024));
            let sibling_uri = "nve-b://sibling-inserted-after".to_owned();
            let cache = ImageCache::shared();
            cache
                .insert_raw_rgba(&short_uri, 16, 16, vec![180u8; 16 * 16 * 4])
                .expect("insert short uri img");
            let short_handle = cache.resolve_cached_handle(&short_uri);
            cache
                .insert_raw_rgba(&long_uri, 16, 16, vec![180u8; 16 * 16 * 4])
                .expect("insert long uri img");
            let long_handle = cache.resolve_cached_handle(&long_uri);
            cache
                .insert_raw_rgba(&sibling_uri, 16, 16, vec![120u8; 16 * 16 * 4])
                .expect("insert sibling img after handle resolution");
            let short_dl = DisplayList {
                ops: vec![PaintOp::DrawImage {
                    rect: LayoutRect {
                        x: 4.0,
                        y: 4.0,
                        width: 32.0,
                        height: 32.0,
                    },
                    radius: 0.0,
                    resolved: short_handle,
                    src: short_uri,
                    fit: ImageFit::Fill,
                }],
            };
            let long_dl = DisplayList {
                ops: vec![PaintOp::DrawImage {
                    rect: LayoutRect {
                        x: 4.0,
                        y: 4.0,
                        width: 32.0,
                        height: 32.0,
                    },
                    radius: 0.0,
                    resolved: long_handle,
                    src: long_uri,
                    fit: ImageFit::Fill,
                }],
            };
            let mut img_cpu = CpuSkiaSurface::new(64, 64).expect("img bench surface");
            img_cpu
                .render_cached_with_plugins(&short_dl, 1.0, ColorRgba::BLACK, None)
                .expect("warm short img");
            img_cpu
                .render_cached_with_plugins(&long_dl, 1.0, ColorRgba::BLACK, None)
                .expect("warm long img");
            img_cpu.reset_cache_stats();
            img_cpu
                .render_cached_with_plugins(&short_dl, 1.0, ColorRgba::BLACK, None)
                .expect("verify short fast hit");
            img_cpu
                .render_cached_with_plugins(&long_dl, 1.0, ColorRgba::BLACK, None)
                .expect("verify long fast hit");
            assert_eq!(
                img_cpu.cache_stats().image_fast_hits,
                2,
                "both pre-resolved handles must hit lock-free fast path after sibling insert"
            );
            c.bench_function("skia_draw_image_warm_16b_uri", |b| {
                b.iter(|| {
                    img_cpu
                        .render_cached_with_plugins(
                            black_box(&short_dl),
                            1.0,
                            ColorRgba::BLACK,
                            None,
                        )
                        .expect("short uri draw");
                    black_box(img_cpu.cache_stats().image_fast_hits)
                });
            });
            c.bench_function("skia_draw_image_warm_16kb_uri", |b| {
                b.iter(|| {
                    img_cpu
                        .render_cached_with_plugins(
                            black_box(&long_dl),
                            1.0,
                            ColorRgba::BLACK,
                            None,
                        )
                        .expect("long uri draw");
                    black_box(img_cpu.cache_stats().image_fast_hits)
                });
            });
        }
    }

    criterion_group!(benches, bench_skia_backend);
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    native::benches();
    criterion::Criterion::default()
        .configure_from_args()
        .final_summary();
}
