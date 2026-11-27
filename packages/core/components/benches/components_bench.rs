//! Criterion micro-benchmarks for `nve-components` hot paths: CSS color/dimension parsing,
//! spring physics integration, 2D affine transforms, and gradient stop sampling.

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::hint::black_box;

    use criterion::{criterion_group, Criterion};
    use nve_abi::ColorRgba;
    use nve_components::{
        affine_concat, affine_map_point, parse_color, parse_dim, sample_gradient_stops,
        GradientStop, MotionPreference, SpringSpec, SpringState, Transform2D,
    };

    fn bench_css_parsers(c: &mut Criterion) {
        let colors = [
            "#2563eb",
            "#fff",
            "rgba(37, 99, 235, 0.75)",
            "rgb(15, 23, 42)",
            "transparent",
            "#10b981cc",
        ];
        let dims = ["auto", "100%", "24px", "12.5px", "320", "50%"];

        c.bench_function("components_parse_color_batch", |b| {
            b.iter(|| {
                let mut acc = 0u32;
                for &raw in black_box(&colors) {
                    if let Some(c) = parse_color(raw) {
                        acc = acc.wrapping_add(c.to_u32());
                    }
                }
                black_box(acc)
            });
        });

        c.bench_function("components_parse_dim_batch", |b| {
            b.iter(|| {
                let mut count = 0usize;
                for &raw in black_box(&dims) {
                    let _ = black_box(parse_dim(raw));
                    count += 1;
                }
                black_box(count)
            });
        });
    }

    fn bench_motion_and_paint(c: &mut Criterion) {
        let spec = SpringSpec::default();
        let stops = [
            GradientStop {
                offset: 0.0,
                color: ColorRgba::rgb(37, 99, 235),
            },
            GradientStop {
                offset: 0.5,
                color: ColorRgba::rgb(139, 92, 246),
            },
            GradientStop {
                offset: 1.0,
                color: ColorRgba::rgb(16, 185, 129),
            },
        ];

        c.bench_function("components_spring_60_steps", |b| {
            b.iter(|| {
                let mut state = SpringState::new(0.0, 1.0);
                for _ in 0..60 {
                    state.step(
                        black_box(&spec),
                        black_box(1.0 / 60.0),
                        MotionPreference::Full,
                    );
                }
                black_box(state.value)
            });
        });

        c.bench_function("components_transform2d_concat_map", |b| {
            let t1 = Transform2D {
                translate_x: 24.0,
                translate_y: 12.0,
                scale_x: 1.25,
                scale_y: 1.25,
                rotate_deg: 15.0,
                ..Transform2D::IDENTITY
            };
            let t2 = Transform2D {
                translate_x: -8.0,
                translate_y: 32.0,
                scale_x: 0.9,
                scale_y: 0.9,
                rotate_deg: -5.0,
                ..Transform2D::IDENTITY
            };
            b.iter(|| {
                let m1 = t1.to_matrix_2d(100.0, 50.0);
                let m2 = t2.to_matrix_2d(100.0, 50.0);
                let m = affine_concat(black_box(&m1), black_box(&m2));
                black_box(affine_map_point(&m, 50.0, 25.0))
            });
        });

        c.bench_function("components_sample_gradient_16", |b| {
            b.iter(|| {
                let mut acc = 0u32;
                for i in 0..16 {
                    let t = i as f32 / 15.0;
                    acc = acc.wrapping_add(sample_gradient_stops(black_box(&stops), t).to_u32());
                }
                black_box(acc)
            });
        });
    }

    criterion_group!(benches, bench_css_parsers, bench_motion_and_paint);
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    native::benches();
    criterion::Criterion::default()
        .configure_from_args()
        .final_summary();
}
