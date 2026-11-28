//! Criterion micro-benchmarks for `nve-svg-skia`: 4-subscanline analytical SVG rasterization and
//! warm `SvgCache` lookup.

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::hint::black_box;

    use criterion::{criterion_group, Criterion};
    use nve_store::{FlatResource, ValidatedWords};
    use nve_svg::{ColorRgba, SvgPicture};
    use nve_svg_skia::{rasterize_svg_picture_view, SvgCache};

    const STAR_PATH: &str = "M12 2 L15.09 8.26 L22 9.27 L17 14.14 L18.18 21.02 L12 17.77 L5.82 21.02 L7 14.14 L2 9.27 L8.91 8.26 Z";

    fn bench_svg_skia_hot_paths(c: &mut Criterion) {
        let pic = SvgPicture::from_paths(24.0, 24.0, &[STAR_PATH]);
        let mut words = Vec::new();
        assert!(pic.write_words(&mut words));
        let vw = ValidatedWords::<SvgPicture>::try_new(&words).expect("valid SvgPicture");
        let view = SvgPicture::view(vw);
        let tint = Some(ColorRgba::rgb(24, 144, 255));

        c.bench_function("svg_skia_rasterize_24x24", |b| {
            b.iter(|| {
                let bmp = rasterize_svg_picture_view(black_box(view), 24, 24, black_box(tint));
                black_box(bmp.premul_rgba.len())
            });
        });

        c.bench_function("svg_skia_rasterize_64x64", |b| {
            b.iter(|| {
                let bmp = rasterize_svg_picture_view(black_box(view), 64, 64, black_box(tint));
                black_box(bmp.premul_rgba.len())
            });
        });

        let cache = SvgCache::new(64, 1024 * 1024);
        let _ = cache.get_or_rasterize_view(view, 48, 48, tint);

        c.bench_function("svg_skia_cache_warm_hit", |b| {
            b.iter(|| {
                let bmp = cache.get_or_rasterize_view(black_box(view), 48, 48, black_box(tint));
                black_box(bmp.width)
            });
        });
    }

    criterion_group!(benches, bench_svg_skia_hot_paths);
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    native::benches();
    criterion::Criterion::default()
        .configure_from_args()
        .final_summary();
}
