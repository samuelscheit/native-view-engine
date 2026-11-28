//! Criterion benchmarks for `nve-dioxus`: Dioxus `VirtualDom` mount into `NodeStore` via
//! `DioxusSurface` and `NVE2` wire encoding.

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::hint::black_box;

    use criterion::{criterion_group, Criterion};
    use dioxus_core::Element;
    use nve_dioxus::rsx;
    use nve_abi::{ColorRgba, FrameId, SurfaceId};
    use nve_dioxus::prelude::*;
    use nve_dioxus::DioxusSurface;

    fn bench_list_app() -> Element {
        rsx! {
            view {
                nve_style: TypedStyle::col().gap(8.0),
                for i in 0..32u32 {
                    view {
                        key: "{i}",
                        nve_style: TypedStyle::new()
                            .padding(8.0)
                            .bg(ColorRgba::rgb(0x1e, 0x29, 0x3b)),
                        text { "Row {i}" }
                    }
                }
            }
        }
    }

    fn bench_dioxus_surface_mount(c: &mut Criterion) {
        let mut buf = Vec::with_capacity(8192);
        c.bench_function("dioxus_surface_mount_32_rows_nve2", |b| {
            b.iter(|| {
                let mut surface = DioxusSurface::new(SurfaceId::ROOT, bench_list_app);
                buf.clear();
                let timings = surface.render_nve2(FrameId(1), &mut buf);
                black_box((timings, buf.len()))
            });
        });
    }

    criterion_group!(benches, bench_dioxus_surface_mount);
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    native::benches();
    criterion::Criterion::default()
        .configure_from_args()
        .final_summary();
}
