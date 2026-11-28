//! Criterion micro-benchmarks for `nve-svg`: SVG path parsing, `FlatResource` flat-word
//! encoding/validation/view iteration, and inline SVG emission.

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::hint::black_box;
    use std::sync::Arc;

    use criterion::{criterion_group, Criterion};
    use nve_store::{FlatResource, ValidatedWords};
    use nve_svg::{
        compute_svg_content_hash, parse_svg_path_data, ColorRgba, FillRule, SvgPicture,
        VectorShape,
    };

    const ICON_PATH: &str = "M12 2 L15.09 8.26 L22 9.27 L17 14.14 L18.18 21.02 L12 17.77 L5.82 21.02 L7 14.14 L2 9.27 L8.91 8.26 Z M6 10 C8 6 16 6 18 10 Q12 16 6 10 Z";

    fn bench_svg_hot_paths(c: &mut Criterion) {
        c.bench_function("svg_parse_path_data", |b| {
            b.iter(|| {
                let cmds = parse_svg_path_data(black_box(ICON_PATH));
                black_box(cmds.len())
            });
        });

        let cmds = Arc::from(parse_svg_path_data(ICON_PATH));
        let pic = SvgPicture::new(
            (0.0, 0.0, 24.0, 24.0),
            vec![
                VectorShape {
                    path: Arc::clone(&cmds),
                    fill: Some(ColorRgba::rgba(32, 128, 255, 255)),
                    stroke: Some(ColorRgba::rgb(12, 24, 48)),
                    stroke_width: 1.5,
                    fill_rule: FillRule::EvenOdd,
                },
                VectorShape {
                    path: cmds,
                    fill: None,
                    stroke: Some(ColorRgba::rgb(255, 255, 255)),
                    stroke_width: 1.0,
                    fill_rule: FillRule::NonZero,
                },
            ],
        );

        let mut words = Vec::with_capacity(256);
        assert!(pic.write_words(&mut words));
        let mut svg_out = String::with_capacity(512);

        c.bench_function("svg_flat_resource_encode_validate_and_view", |b| {
            b.iter(|| {
                words.clear();
                let ok = black_box(&pic).write_words(&mut words);
                debug_assert!(ok);
                let vw = ValidatedWords::<SvgPicture>::try_new(&words).expect("valid SvgPicture");
                let view = SvgPicture::view(vw);
                svg_out.clear();
                view.write_svg_children_into(None, &mut svg_out);
                black_box((view.content_hash(), svg_out.len()))
            });
        });

        c.bench_function("svg_compute_content_hash", |b| {
            b.iter(|| {
                black_box(compute_svg_content_hash(
                    black_box(pic.view_box),
                    black_box(&pic.shapes),
                ))
            });
        });
    }

    criterion_group!(benches, bench_svg_hot_paths);
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    native::benches();
    criterion::Criterion::default()
        .configure_from_args()
        .final_summary();
}
