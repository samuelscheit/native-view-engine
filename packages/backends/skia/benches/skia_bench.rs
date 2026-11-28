//! Criterion benchmarks for `nve-backend-skia`: `SkiaBackend::commit` display-list build,
//! zero-C++ software `PixelBuffer` rasterization, and `data:image/png;base64,...` decoding.

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::hint::black_box;

    use criterion::{criterion_group, Criterion};
    use nve_abi::{ColorRgba, SurfaceId};
    use nve_backend_skia::raster::{rasterize_display_list, PixelBuffer};
    use nve_backend_skia::{decode_image_uri, SkiaBackend};
    use nve_tables::tables::{Layout, TextContent, Visual};
    use nve_engine::{StoreBackend, Viewport};
    use nve_store::{ChangeSet, Length, NodeId, NodeStore, Rgba};

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
                black_box(backend.display_list().ops.len())
            });
        });

        let list = backend.display_list().clone();
        let mut fb = PixelBuffer::new(320, 240);
        c.bench_function("skia_software_rasterize_320x240", |b| {
            b.iter(|| {
                rasterize_display_list(black_box(&list), &mut fb, 1.0, ColorRgba::BLACK);
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
